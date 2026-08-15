use crate::reexecution::ReexecExecutedTxArtifacts;
use mc_db::preconfirmed::PreconfirmedExecutedTransaction;
use mp_block::commitments::{compute_event_commitment, compute_receipt_commitment, compute_transaction_commitment};
use mp_chain_config::StarknetVersion;
use mp_convert::Felt;
use mp_receipt::{Event, ExecutionResources, TransactionReceipt};
use mp_state_update::StateDiff;
use mp_transactions::Transaction;
use std::collections::{BTreeMap, BTreeSet};

const MAX_VALUE_LEN: usize = 256;

#[derive(Debug, Clone)]
pub struct TransactionOutputComparisonConfig {
    pub fee_token_addresses: BTreeSet<Felt>,
    pub fee_transfer_selector: Felt,
    pub sequencer_address: Felt,
    pub protocol_version: StarknetVersion,
    pub chain_id: Felt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum MismatchCategory {
    TransactionAlignment,
    ExecutionResult,
    StateUpdate,
    Event,
    Message,
    ReceiptMetadata,
    Fee,
    Resource,
    Commitment,
}

impl MismatchCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TransactionAlignment => "transaction_alignment",
            Self::ExecutionResult => "execution_result",
            Self::StateUpdate => "state_update",
            Self::Event => "event",
            Self::Message => "message",
            Self::ReceiptMetadata => "receipt_metadata",
            Self::Fee => "fee",
            Self::Resource => "resource",
            Self::Commitment => "commitment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum MismatchPolicy {
    Strict,
    Allowed,
    Warning,
    Diagnostic,
}

impl MismatchPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Allowed => "allowed",
            Self::Warning => "warning",
            Self::Diagnostic => "diagnostic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FieldMismatch {
    pub category: MismatchCategory,
    pub policy: MismatchPolicy,
    pub transaction_hash: Option<Felt>,
    pub transaction_index: Option<usize>,
    pub field_path: String,
    pub execution_box_value: String,
    pub blockifier_value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CandidateCommitments {
    pub transaction: Felt,
    pub receipt: Felt,
    pub event: Felt,
    pub state_diff: Felt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct CandidateCommitmentComparison {
    pub execution_box: CandidateCommitments,
    pub blockifier: CandidateCommitments,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct BlockComparisonReport {
    pub strict_mismatches: Vec<FieldMismatch>,
    pub allowed_mismatches: Vec<FieldMismatch>,
    pub resource_warnings: Vec<FieldMismatch>,
    pub diagnostics: Vec<FieldMismatch>,
    pub affected_transaction_hashes: BTreeSet<Felt>,
    pub execution_box_transaction_count: usize,
    pub blockifier_transaction_count: usize,
    pub canonical_transaction_count: usize,
    pub paired_transaction_count: usize,
    pub commitments: CandidateCommitmentComparison,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputMismatchSummary {
    pub categories: BTreeSet<MismatchCategory>,
    pub mismatch_count: usize,
    pub affected_transaction_hashes: BTreeSet<Felt>,
}

impl std::fmt::Display for OutputMismatchSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let categories = self.categories.iter().map(|category| category.as_str()).collect::<Vec<_>>().join(",");
        write!(
            f,
            "OutputMismatch(categories=[{categories}], mismatches={}, affected_txs={})",
            self.mismatch_count,
            self.affected_transaction_hashes.len()
        )
    }
}

impl BlockComparisonReport {
    pub fn has_strict_mismatch(&self) -> bool {
        !self.strict_mismatches.is_empty()
    }

    pub fn has_strict_category(&self, category: MismatchCategory) -> bool {
        self.strict_mismatches.iter().any(|mismatch| mismatch.category == category)
    }

    pub fn strict_summary(&self) -> OutputMismatchSummary {
        OutputMismatchSummary {
            categories: self.strict_mismatches.iter().map(|mismatch| mismatch.category).collect(),
            mismatch_count: self.strict_mismatches.len(),
            affected_transaction_hashes: self
                .strict_mismatches
                .iter()
                .filter_map(|mismatch| mismatch.transaction_hash)
                .collect(),
        }
    }

    pub fn mismatch_counts(&self) -> BTreeMap<(MismatchCategory, MismatchPolicy), u64> {
        self.iter_mismatches().fold(BTreeMap::new(), |mut counts, mismatch| {
            *counts.entry((mismatch.category, mismatch.policy)).or_default() += 1;
            counts
        })
    }

    pub fn iter_mismatches(&self) -> impl Iterator<Item = &FieldMismatch> {
        self.strict_mismatches
            .iter()
            .chain(&self.allowed_mismatches)
            .chain(&self.resource_warnings)
            .chain(&self.diagnostics)
    }

    pub fn push(&mut self, mismatch: FieldMismatch) {
        if let Some(hash) = mismatch.transaction_hash {
            self.affected_transaction_hashes.insert(hash);
        }
        match mismatch.policy {
            MismatchPolicy::Strict => self.strict_mismatches.push(mismatch),
            MismatchPolicy::Allowed => self.allowed_mismatches.push(mismatch),
            MismatchPolicy::Warning => self.resource_warnings.push(mismatch),
            MismatchPolicy::Diagnostic => self.diagnostics.push(mismatch),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn compare_transaction_outputs(
    execution_box_rows: &[PreconfirmedExecutedTransaction],
    blockifier_rows: &[ReexecExecutedTxArtifacts],
    canonical_transaction_hashes: &[Felt],
    execution_box_state_diff: &StateDiff,
    blockifier_state_diff: &StateDiff,
    aggregate_state_diff_is_strict_mismatch: bool,
    config: &TransactionOutputComparisonConfig,
) -> BlockComparisonReport {
    let mut report = BlockComparisonReport {
        execution_box_transaction_count: execution_box_rows.len(),
        blockifier_transaction_count: blockifier_rows.len(),
        canonical_transaction_count: canonical_transaction_hashes.len(),
        ..Default::default()
    };

    let canonical_index = index_canonical_hashes(canonical_transaction_hashes, &mut report);
    let execution_box_by_hash = index_execution_box_rows(execution_box_rows, &canonical_index, &mut report);
    let blockifier_by_hash = index_blockifier_rows(blockifier_rows, &canonical_index, &mut report);

    compare_membership("execution_box", &canonical_index, &execution_box_by_hash, &mut report);
    compare_membership("blockifier", &canonical_index, &blockifier_by_hash, &mut report);

    compare_order(
        "execution_box",
        execution_box_rows.iter().map(|row| *row.transaction.receipt.transaction_hash()).collect(),
        canonical_transaction_hashes,
        &mut report,
    );
    compare_order(
        "blockifier",
        blockifier_rows.iter().map(|row| *row.receipt.transaction_hash()).collect(),
        canonical_transaction_hashes,
        &mut report,
    );

    for (&transaction_hash, &transaction_index) in &canonical_index {
        let (Some(execution_box), Some(blockifier)) =
            (execution_box_by_hash.get(&transaction_hash), blockifier_by_hash.get(&transaction_hash))
        else {
            continue;
        };
        report.paired_transaction_count += 1;
        compare_transaction(transaction_hash, transaction_index, execution_box, blockifier, config, &mut report);
    }

    compare_commitments(
        execution_box_rows,
        blockifier_rows,
        &execution_box_by_hash,
        execution_box_state_diff,
        blockifier_state_diff,
        aggregate_state_diff_is_strict_mismatch,
        config,
        &mut report,
    );
    report
}

fn index_canonical_hashes(hashes: &[Felt], report: &mut BlockComparisonReport) -> BTreeMap<Felt, usize> {
    let mut index = BTreeMap::new();
    for (transaction_index, &transaction_hash) in hashes.iter().enumerate() {
        if let Some(first_index) = index.insert(transaction_hash, transaction_index) {
            report.push(mismatch(
                MismatchCategory::TransactionAlignment,
                MismatchPolicy::Strict,
                Some(transaction_hash),
                Some(transaction_index),
                "canonical_transactions.duplicate_hash",
                first_index,
                transaction_index,
            ));
        }
    }
    index
}

fn index_execution_box_rows<'a>(
    rows: &'a [PreconfirmedExecutedTransaction],
    canonical_index: &BTreeMap<Felt, usize>,
    report: &mut BlockComparisonReport,
) -> BTreeMap<Felt, &'a PreconfirmedExecutedTransaction> {
    let mut index = BTreeMap::new();
    for row in rows {
        let hash = *row.transaction.receipt.transaction_hash();
        if index.insert(hash, row).is_some() {
            report.push(mismatch(
                MismatchCategory::TransactionAlignment,
                MismatchPolicy::Strict,
                Some(hash),
                canonical_index.get(&hash).copied(),
                "execution_box.transactions.duplicate_hash",
                "unique",
                "duplicate",
            ));
        }
    }
    index
}

fn index_blockifier_rows<'a>(
    rows: &'a [ReexecExecutedTxArtifacts],
    canonical_index: &BTreeMap<Felt, usize>,
    report: &mut BlockComparisonReport,
) -> BTreeMap<Felt, &'a ReexecExecutedTxArtifacts> {
    let mut index = BTreeMap::new();
    for row in rows {
        let hash = *row.receipt.transaction_hash();
        if index.insert(hash, row).is_some() {
            report.push(mismatch(
                MismatchCategory::TransactionAlignment,
                MismatchPolicy::Strict,
                Some(hash),
                canonical_index.get(&hash).copied(),
                "blockifier.transactions.duplicate_hash",
                "unique",
                "duplicate",
            ));
        }
    }
    index
}

fn compare_membership<T>(
    source: &str,
    canonical: &BTreeMap<Felt, usize>,
    actual: &BTreeMap<Felt, T>,
    report: &mut BlockComparisonReport,
) {
    for (&hash, &transaction_index) in canonical {
        if !actual.contains_key(&hash) {
            report.push(mismatch(
                MismatchCategory::TransactionAlignment,
                MismatchPolicy::Strict,
                Some(hash),
                Some(transaction_index),
                format!("{source}.transactions.membership"),
                "present",
                "missing",
            ));
        }
    }
    for &hash in actual.keys() {
        if !canonical.contains_key(&hash) {
            report.push(mismatch(
                MismatchCategory::TransactionAlignment,
                MismatchPolicy::Strict,
                Some(hash),
                None,
                format!("{source}.transactions.membership"),
                "absent",
                "extra",
            ));
        }
    }
}

fn compare_order(source: &str, actual: Vec<Felt>, canonical: &[Felt], report: &mut BlockComparisonReport) {
    if actual != canonical {
        report.push(mismatch(
            MismatchCategory::TransactionAlignment,
            MismatchPolicy::Diagnostic,
            None,
            None,
            format!("{source}.transactions.order"),
            canonical,
            actual,
        ));
    }
}

fn compare_transaction(
    transaction_hash: Felt,
    transaction_index: usize,
    execution_box: &PreconfirmedExecutedTransaction,
    blockifier: &ReexecExecutedTxArtifacts,
    config: &TransactionOutputComparisonConfig,
    report: &mut BlockComparisonReport,
) {
    let execution_box_receipt = &execution_box.transaction.receipt;
    let blockifier_receipt = &blockifier.receipt;
    compare_strict(
        transaction_variant(&execution_box.transaction.transaction),
        receipt_variant(blockifier_receipt),
        MismatchCategory::TransactionAlignment,
        "transaction.variant",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_strict(
        receipt_variant(execution_box_receipt),
        receipt_variant(blockifier_receipt),
        MismatchCategory::ReceiptMetadata,
        "receipt.variant",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_strict(
        execution_box_receipt.transaction_hash(),
        blockifier_receipt.transaction_hash(),
        MismatchCategory::ReceiptMetadata,
        "receipt.transaction_hash",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_strict(
        execution_box_receipt.execution_result(),
        blockifier_receipt.execution_result(),
        MismatchCategory::ExecutionResult,
        "receipt.execution_result",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_strict(
        execution_box_receipt.actual_fee().unit,
        blockifier_receipt.actual_fee().unit,
        MismatchCategory::Fee,
        "receipt.actual_fee.unit",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_allowed(
        execution_box_receipt.actual_fee().amount,
        blockifier_receipt.actual_fee().amount,
        MismatchCategory::Fee,
        "receipt.actual_fee.amount",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_receipt_metadata(execution_box_receipt, blockifier_receipt, transaction_hash, transaction_index, report);
    compare_events(
        execution_box_receipt,
        blockifier_receipt,
        transaction_fee_payer(&execution_box.transaction.transaction, execution_box_receipt),
        transaction_hash,
        transaction_index,
        config,
        report,
    );
    compare_messages(execution_box_receipt, blockifier_receipt, transaction_hash, transaction_index, report);
    compare_resources(
        execution_box_receipt.execution_resources(),
        blockifier_receipt.execution_resources(),
        transaction_hash,
        transaction_index,
        report,
    );
    compare_diagnostic(
        &execution_box.state_diff.nonces,
        &blockifier.tx_state_update.nonces,
        MismatchCategory::StateUpdate,
        "transaction_state_diff.nonces",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_diagnostic(
        &execution_box.state_diff.storage_diffs,
        &blockifier.tx_state_update.storage_diffs,
        MismatchCategory::StateUpdate,
        "transaction_state_diff.storage_diffs",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_diagnostic(
        &execution_box.state_diff.declared_classes,
        &blockifier.tx_state_update.declared_classes,
        MismatchCategory::StateUpdate,
        "transaction_state_diff.declared_classes",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_diagnostic(
        &execution_box.state_diff.contract_class_hashes,
        &blockifier.tx_state_update.contract_class_hashes,
        MismatchCategory::StateUpdate,
        "transaction_state_diff.contract_class_hashes",
        transaction_hash,
        transaction_index,
        report,
    );
}

fn compare_receipt_metadata(
    execution_box: &TransactionReceipt,
    blockifier: &TransactionReceipt,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    compare_strict(
        execution_box.contract_address(),
        blockifier.contract_address(),
        MismatchCategory::ReceiptMetadata,
        "receipt.contract_address",
        transaction_hash,
        transaction_index,
        report,
    );
    compare_strict(
        execution_box.as_l1_handler().map(|receipt| receipt.message_hash),
        blockifier.as_l1_handler().map(|receipt| receipt.message_hash),
        MismatchCategory::ReceiptMetadata,
        "receipt.l1_handler_message_hash",
        transaction_hash,
        transaction_index,
        report,
    );
}

fn compare_events(
    execution_box: &TransactionReceipt,
    blockifier: &TransactionReceipt,
    fee_payer: Option<Felt>,
    transaction_hash: Felt,
    transaction_index: usize,
    config: &TransactionOutputComparisonConfig,
    report: &mut BlockComparisonReport,
) {
    let execution_box_events = execution_box.events();
    let blockifier_events = blockifier.events();
    compare_strict(
        execution_box_events.len(),
        blockifier_events.len(),
        MismatchCategory::Event,
        "receipt.events.len",
        transaction_hash,
        transaction_index,
        report,
    );
    for (event_index, (execution_box_event, blockifier_event)) in
        execution_box_events.iter().zip(blockifier_events).enumerate()
    {
        let path = |field: &str| format!("receipt.events[{event_index}].{field}");
        compare_strict(
            execution_box_event.from_address,
            blockifier_event.from_address,
            MismatchCategory::Event,
            path("from_address"),
            transaction_hash,
            transaction_index,
            report,
        );
        compare_strict(
            execution_box_event.keys.len(),
            blockifier_event.keys.len(),
            MismatchCategory::Event,
            path("keys.len"),
            transaction_hash,
            transaction_index,
            report,
        );
        compare_strict(
            &execution_box_event.keys,
            &blockifier_event.keys,
            MismatchCategory::Event,
            path("keys"),
            transaction_hash,
            transaction_index,
            report,
        );
        compare_strict(
            execution_box_event.data.len(),
            blockifier_event.data.len(),
            MismatchCategory::Event,
            path("data.len"),
            transaction_hash,
            transaction_index,
            report,
        );

        let fee_transfer_layout = fee_transfer_layout(execution_box_event, config, fee_payer).filter(|layout| {
            Some(*layout) == fee_transfer_layout(blockifier_event, config, fee_payer)
                && execution_box_event.from_address == blockifier_event.from_address
                && execution_box_event.keys == blockifier_event.keys
        });
        for (data_index, (&execution_box_value, &blockifier_value)) in
            execution_box_event.data.iter().zip(&blockifier_event.data).enumerate()
        {
            let field_path = path(&format!("data[{data_index}]"));
            let is_fee_amount = matches!(
                (fee_transfer_layout, data_index),
                (Some(FeeTransferLayout::Legacy), 2 | 3) | (Some(FeeTransferLayout::CairoOne), 0 | 1)
            );
            if is_fee_amount {
                compare_allowed(
                    execution_box_value,
                    blockifier_value,
                    MismatchCategory::Fee,
                    field_path,
                    transaction_hash,
                    transaction_index,
                    report,
                );
            } else {
                compare_strict(
                    execution_box_value,
                    blockifier_value,
                    MismatchCategory::Event,
                    field_path,
                    transaction_hash,
                    transaction_index,
                    report,
                );
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FeeTransferLayout {
    Legacy,
    CairoOne,
}

fn fee_transfer_layout(
    event: &Event,
    config: &TransactionOutputComparisonConfig,
    fee_payer: Option<Felt>,
) -> Option<FeeTransferLayout> {
    let fee_payer = fee_payer?;
    if !config.fee_token_addresses.contains(&event.from_address) {
        return None;
    }
    if event.keys.as_slice() == [config.fee_transfer_selector]
        && event.data.as_slice().get(..2) == Some([fee_payer, config.sequencer_address].as_slice())
        && event.data.len() == 4
    {
        Some(FeeTransferLayout::Legacy)
    } else if event.keys.as_slice() == [config.fee_transfer_selector, fee_payer, config.sequencer_address]
        && event.data.len() == 2
    {
        Some(FeeTransferLayout::CairoOne)
    } else {
        None
    }
}

fn transaction_fee_payer(transaction: &Transaction, receipt: &TransactionReceipt) -> Option<Felt> {
    match transaction {
        Transaction::Invoke(tx) => Some(*tx.sender_address()),
        Transaction::Declare(tx) => Some(*tx.sender_address()),
        Transaction::DeployAccount(_) => receipt.contract_address().copied(),
        Transaction::L1Handler(_) | Transaction::Deploy(_) => None,
    }
}

fn compare_messages(
    execution_box: &TransactionReceipt,
    blockifier: &TransactionReceipt,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    let execution_box_messages = execution_box.messages_sent();
    let blockifier_messages = blockifier.messages_sent();
    compare_strict(
        execution_box_messages.len(),
        blockifier_messages.len(),
        MismatchCategory::Message,
        "receipt.messages_sent.len",
        transaction_hash,
        transaction_index,
        report,
    );
    for (message_index, (execution_box_message, blockifier_message)) in
        execution_box_messages.iter().zip(blockifier_messages).enumerate()
    {
        compare_strict(
            execution_box_message.from_address,
            blockifier_message.from_address,
            MismatchCategory::Message,
            format!("receipt.messages_sent[{message_index}].from_address"),
            transaction_hash,
            transaction_index,
            report,
        );
        compare_strict(
            execution_box_message.to_address,
            blockifier_message.to_address,
            MismatchCategory::Message,
            format!("receipt.messages_sent[{message_index}].to_address"),
            transaction_hash,
            transaction_index,
            report,
        );
        compare_strict(
            &execution_box_message.payload,
            &blockifier_message.payload,
            MismatchCategory::Message,
            format!("receipt.messages_sent[{message_index}].payload"),
            transaction_hash,
            transaction_index,
            report,
        );
    }
}

fn compare_resources(
    execution_box: &ExecutionResources,
    blockifier: &ExecutionResources,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    macro_rules! diagnostic_resource {
        ($field:ident) => {
            compare_diagnostic(
                execution_box.$field,
                blockifier.$field,
                MismatchCategory::Resource,
                concat!("receipt.execution_resources.", stringify!($field)),
                transaction_hash,
                transaction_index,
                report,
            )
        };
    }
    diagnostic_resource!(steps);
    diagnostic_resource!(memory_holes);
    diagnostic_resource!(range_check_builtin_applications);
    diagnostic_resource!(pedersen_builtin_applications);
    diagnostic_resource!(poseidon_builtin_applications);
    diagnostic_resource!(ec_op_builtin_applications);
    diagnostic_resource!(ecdsa_builtin_applications);
    diagnostic_resource!(bitwise_builtin_applications);
    diagnostic_resource!(keccak_builtin_applications);
    diagnostic_resource!(segment_arena_builtin);
    for (field, execution_box, blockifier) in [
        ("data_availability.l1_gas", execution_box.data_availability.l1_gas, blockifier.data_availability.l1_gas),
        (
            "data_availability.l1_data_gas",
            execution_box.data_availability.l1_data_gas,
            blockifier.data_availability.l1_data_gas,
        ),
        ("data_availability.l2_gas", execution_box.data_availability.l2_gas, blockifier.data_availability.l2_gas),
        ("total_gas_consumed.l1_gas", execution_box.total_gas_consumed.l1_gas, blockifier.total_gas_consumed.l1_gas),
        (
            "total_gas_consumed.l1_data_gas",
            execution_box.total_gas_consumed.l1_data_gas,
            blockifier.total_gas_consumed.l1_data_gas,
        ),
        ("total_gas_consumed.l2_gas", execution_box.total_gas_consumed.l2_gas, blockifier.total_gas_consumed.l2_gas),
    ] {
        compare_diagnostic(
            execution_box,
            blockifier,
            MismatchCategory::Resource,
            format!("receipt.execution_resources.{field}"),
            transaction_hash,
            transaction_index,
            report,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_commitments(
    execution_box_rows: &[PreconfirmedExecutedTransaction],
    blockifier_rows: &[ReexecExecutedTxArtifacts],
    execution_box_by_hash: &BTreeMap<Felt, &PreconfirmedExecutedTransaction>,
    execution_box_state_diff: &StateDiff,
    blockifier_state_diff: &StateDiff,
    aggregate_state_diff_is_strict_mismatch: bool,
    config: &TransactionOutputComparisonConfig,
    report: &mut BlockComparisonReport,
) {
    let version = config.protocol_version;
    let execution_box = candidate_commitments(
        execution_box_rows.iter().map(|row| (&row.transaction.transaction, &row.transaction.receipt)),
        execution_box_state_diff,
        version,
        config.chain_id,
    );
    let blockifier = candidate_commitments(
        blockifier_rows.iter().filter_map(|row| {
            execution_box_by_hash
                .get(row.receipt.transaction_hash())
                .map(|execution_box| (&execution_box.transaction.transaction, &row.receipt))
        }),
        blockifier_state_diff,
        version,
        config.chain_id,
    );
    report.commitments =
        CandidateCommitmentComparison { execution_box: execution_box.clone(), blockifier: blockifier.clone() };

    let transaction_is_strict =
        report.strict_mismatches.iter().any(|mismatch| mismatch.category == MismatchCategory::TransactionAlignment);
    let receipt_is_strict = report.strict_mismatches.iter().any(|mismatch| {
        matches!(
            mismatch.category,
            MismatchCategory::TransactionAlignment
                | MismatchCategory::ExecutionResult
                | MismatchCategory::Message
                | MismatchCategory::ReceiptMetadata
                | MismatchCategory::Fee
        )
    });
    let event_is_strict = report
        .strict_mismatches
        .iter()
        .any(|mismatch| matches!(mismatch.category, MismatchCategory::TransactionAlignment | MismatchCategory::Event));
    compare_commitment(
        execution_box.transaction,
        blockifier.transaction,
        "commitments.transaction",
        transaction_is_strict,
        report,
    );
    compare_commitment(execution_box.receipt, blockifier.receipt, "commitments.receipt", receipt_is_strict, report);
    compare_commitment(execution_box.event, blockifier.event, "commitments.event", event_is_strict, report);
    compare_commitment(
        execution_box.state_diff,
        blockifier.state_diff,
        "commitments.state_diff",
        aggregate_state_diff_is_strict_mismatch,
        report,
    );
}

fn candidate_commitments<'a>(
    rows: impl IntoIterator<Item = (&'a Transaction, &'a TransactionReceipt)>,
    state_diff: &StateDiff,
    version: StarknetVersion,
    chain_id: Felt,
) -> CandidateCommitments {
    let rows: Vec<_> = rows.into_iter().collect();
    let transaction = compute_transaction_commitment(
        rows.iter().map(|(transaction, _)| {
            let hash = transaction.compute_hash(chain_id, version, false);
            transaction.compute_hash_with_signature(hash, version)
        }),
        version,
    );
    let receipt = compute_receipt_commitment(rows.iter().map(|(_, receipt)| receipt.compute_hash()), version);
    let event = compute_event_commitment(
        rows.iter().flat_map(|(_, receipt)| {
            receipt.events().iter().map(move |event| event.compute_hash(*receipt.transaction_hash(), version))
        }),
        version,
    );
    CandidateCommitments { transaction, receipt, event, state_diff: state_diff.compute_hash() }
}

fn compare_commitment(
    execution_box: Felt,
    blockifier: Felt,
    field_path: &str,
    strict_leaf_mismatch: bool,
    report: &mut BlockComparisonReport,
) {
    if execution_box != blockifier {
        report.push(mismatch(
            MismatchCategory::Commitment,
            if strict_leaf_mismatch { MismatchPolicy::Strict } else { MismatchPolicy::Diagnostic },
            None,
            None,
            field_path,
            execution_box,
            blockifier,
        ));
    }
}

fn transaction_variant(transaction: &Transaction) -> &'static str {
    match transaction {
        Transaction::Invoke(_) => "invoke",
        Transaction::L1Handler(_) => "l1_handler",
        Transaction::Declare(_) => "declare",
        Transaction::Deploy(_) => "deploy",
        Transaction::DeployAccount(_) => "deploy_account",
    }
}

fn receipt_variant(receipt: &TransactionReceipt) -> &'static str {
    match receipt {
        TransactionReceipt::Invoke(_) => "invoke",
        TransactionReceipt::L1Handler(_) => "l1_handler",
        TransactionReceipt::Declare(_) => "declare",
        TransactionReceipt::Deploy(_) => "deploy",
        TransactionReceipt::DeployAccount(_) => "deploy_account",
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_strict<T: PartialEq + std::fmt::Debug>(
    execution_box: T,
    blockifier: T,
    category: MismatchCategory,
    field_path: impl Into<String>,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    compare_with_policy(
        execution_box,
        blockifier,
        category,
        MismatchPolicy::Strict,
        field_path,
        transaction_hash,
        transaction_index,
        report,
    );
}

#[allow(clippy::too_many_arguments)]
fn compare_allowed<T: PartialEq + std::fmt::Debug>(
    execution_box: T,
    blockifier: T,
    category: MismatchCategory,
    field_path: impl Into<String>,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    compare_with_policy(
        execution_box,
        blockifier,
        category,
        MismatchPolicy::Allowed,
        field_path,
        transaction_hash,
        transaction_index,
        report,
    );
}

#[allow(clippy::too_many_arguments)]
fn compare_diagnostic<T: PartialEq + std::fmt::Debug>(
    execution_box: T,
    blockifier: T,
    category: MismatchCategory,
    field_path: impl Into<String>,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    compare_with_policy(
        execution_box,
        blockifier,
        category,
        MismatchPolicy::Diagnostic,
        field_path,
        transaction_hash,
        transaction_index,
        report,
    );
}

#[allow(clippy::too_many_arguments)]
fn compare_with_policy<T: PartialEq + std::fmt::Debug>(
    execution_box: T,
    blockifier: T,
    category: MismatchCategory,
    policy: MismatchPolicy,
    field_path: impl Into<String>,
    transaction_hash: Felt,
    transaction_index: usize,
    report: &mut BlockComparisonReport,
) {
    if execution_box != blockifier {
        report.push(mismatch(
            category,
            policy,
            Some(transaction_hash),
            Some(transaction_index),
            field_path,
            execution_box,
            blockifier,
        ));
    }
}

fn mismatch(
    category: MismatchCategory,
    policy: MismatchPolicy,
    transaction_hash: Option<Felt>,
    transaction_index: Option<usize>,
    field_path: impl Into<String>,
    execution_box: impl std::fmt::Debug,
    blockifier: impl std::fmt::Debug,
) -> FieldMismatch {
    FieldMismatch {
        category,
        policy,
        transaction_hash,
        transaction_index,
        field_path: field_path.into(),
        execution_box_value: short_debug(&execution_box),
        blockifier_value: short_debug(&blockifier),
    }
}

fn short_debug(value: &impl std::fmt::Debug) -> String {
    let value = format!("{value:?}");
    if value.len() <= MAX_VALUE_LEN {
        value
    } else {
        format!("{}…", value.chars().take(MAX_VALUE_LEN).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_block::TransactionWithReceipt;
    use mp_receipt::{Event, ExecutionResult, FeePayment, GasVector, InvokeTransactionReceipt, MsgToL1, PriceUnit};
    use mp_state_update::TransactionStateUpdate;
    use mp_transactions::{validated::TxTimestamp, InvokeTransaction, InvokeTransactionV1};

    const TRANSACTION_EXECUTED_SELECTOR: Felt =
        Felt::from_hex_unchecked("0x1dcde06aabdbca2f80aa51392b345d7549d7757aa855f7e37f5d335ac8243b1");
    const TRANSFER_SELECTOR: Felt =
        Felt::from_hex_unchecked("0x99cd8bde557814842a3121e8ddfd433a539b8c9f14bf31ebf108d12e6196e9");
    const BALANCE_UPDATE_SELECTOR: Felt =
        Felt::from_hex_unchecked("0x4b5ebfccc3c257d31088bfaba23d3429e791ec8d13412658b6cc38132a04c6");
    const FEE_TOKEN: Felt =
        Felt::from_hex_unchecked("0x47adc7dee88eec362d71a52c25d40559a921434b2d90e75b6a4a6e4e9fb9ab1");

    fn config() -> TransactionOutputComparisonConfig {
        TransactionOutputComparisonConfig {
            fee_token_addresses: BTreeSet::from([FEE_TOKEN]),
            fee_transfer_selector: TRANSFER_SELECTOR,
            sequencer_address: Felt::from(12u64),
            protocol_version: StarknetVersion::V0_13_2,
            chain_id: Felt::from_bytes_be_slice(b"SN_MAIN"),
        }
    }

    fn receipt(hash: Felt) -> TransactionReceipt {
        TransactionReceipt::Invoke(InvokeTransactionReceipt { transaction_hash: hash, ..Default::default() })
    }

    fn row(hash: Felt, receipt: TransactionReceipt) -> PreconfirmedExecutedTransaction {
        row_with_sender(hash, receipt, Felt::ZERO)
    }

    fn row_with_sender(_hash: Felt, receipt: TransactionReceipt, sender: Felt) -> PreconfirmedExecutedTransaction {
        PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: Transaction::Invoke(InvokeTransaction::V1(InvokeTransactionV1 {
                    sender_address: sender,
                    ..Default::default()
                })),
                receipt,
            },
            state_diff: TransactionStateUpdate::default(),
            declared_class: None,
            arrived_at: TxTimestamp(0),
            paid_fee_on_l1: None,
        }
    }

    fn artifact(receipt: TransactionReceipt) -> ReexecExecutedTxArtifacts {
        ReexecExecutedTxArtifacts { receipt, tx_state_update: TransactionStateUpdate::default() }
    }

    fn compare(
        execution_box_rows: &[PreconfirmedExecutedTransaction],
        blockifier_rows: &[ReexecExecutedTxArtifacts],
        canonical: &[Felt],
    ) -> BlockComparisonReport {
        compare_transaction_outputs(
            execution_box_rows,
            blockifier_rows,
            canonical,
            &StateDiff::default(),
            &StateDiff::default(),
            false,
            &config(),
        )
    }

    #[test]
    fn pairs_transactions_by_hash_and_treats_physical_order_as_diagnostic() {
        let hash_1 = Felt::ONE;
        let hash_2 = Felt::TWO;
        let execution_box = vec![row(hash_2, receipt(hash_2)), row(hash_1, receipt(hash_1))];
        let blockifier = vec![artifact(receipt(hash_1)), artifact(receipt(hash_2))];

        let report = compare(&execution_box, &blockifier, &[hash_1, hash_2]);

        assert_eq!(report.paired_transaction_count, 2);
        assert!(report.strict_mismatches.is_empty());
        assert!(report.diagnostics.iter().any(|mismatch| mismatch.field_path == "execution_box.transactions.order"));
    }

    #[test]
    fn missing_duplicate_and_extra_transactions_are_strict() {
        let hash_1 = Felt::ONE;
        let hash_2 = Felt::TWO;
        let hash_3 = Felt::THREE;
        let execution_box = vec![row(hash_1, receipt(hash_1)), row(hash_1, receipt(hash_1))];
        let blockifier = vec![artifact(receipt(hash_1)), artifact(receipt(hash_3))];

        let report = compare(&execution_box, &blockifier, &[hash_1, hash_2]);

        assert!(report.has_strict_mismatch());
        assert!(report.strict_mismatches.iter().all(|mismatch| matches!(
            mismatch.category,
            MismatchCategory::TransactionAlignment | MismatchCategory::Commitment
        )));
    }

    #[test]
    fn execution_result_messages_and_fee_unit_are_strict() {
        let hash = Felt::ONE;
        let mut execution_box_receipt = receipt(hash);
        let mut blockifier_receipt = receipt(hash);
        if let TransactionReceipt::Invoke(receipt) = &mut execution_box_receipt {
            receipt.execution_result = ExecutionResult::Reverted { reason: "left".into() };
            receipt.messages_sent =
                vec![MsgToL1 { from_address: Felt::ONE, to_address: Felt::TWO, payload: vec![Felt::THREE] }];
            receipt.actual_fee.unit = PriceUnit::Wei;
        }
        if let TransactionReceipt::Invoke(receipt) = &mut blockifier_receipt {
            receipt.execution_result = ExecutionResult::Reverted { reason: "right".into() };
            receipt.messages_sent =
                vec![MsgToL1 { from_address: Felt::ONE, to_address: Felt::TWO, payload: vec![Felt::from(4u64)] }];
            receipt.actual_fee.unit = PriceUnit::Fri;
        }

        let report = compare(&[row(hash, execution_box_receipt)], &[artifact(blockifier_receipt)], &[hash]);

        for category in [MismatchCategory::ExecutionResult, MismatchCategory::Message, MismatchCategory::Fee] {
            assert!(report.strict_mismatches.iter().any(|mismatch| mismatch.category == category));
        }
    }

    #[test]
    fn transaction_and_receipt_variants_are_strict() {
        let hash = Felt::ONE;
        let mut blockifier_receipt = TransactionReceipt::Declare(Default::default());
        if let TransactionReceipt::Declare(receipt) = &mut blockifier_receipt {
            receipt.transaction_hash = hash;
        }

        let report = compare(&[row(hash, receipt(hash))], &[artifact(blockifier_receipt)], &[hash]);

        assert!(report.strict_mismatches.iter().any(|mismatch| mismatch.field_path == "transaction.variant"));
        assert!(report.strict_mismatches.iter().any(|mismatch| {
            mismatch.category == MismatchCategory::ReceiptMetadata && mismatch.field_path == "receipt.variant"
        }));
    }

    #[test]
    fn fee_and_receipt_resource_differences_are_reported_but_not_strict() {
        let hash = Felt::ONE;
        let mut execution_box_receipt = receipt(hash);
        let mut blockifier_receipt = receipt(hash);
        if let TransactionReceipt::Invoke(receipt) = &mut execution_box_receipt {
            receipt.actual_fee = FeePayment { amount: Felt::from(10u64), unit: PriceUnit::Wei };
            receipt.execution_resources.steps = 100;
            receipt.execution_resources.memory_holes = 2;
            receipt.execution_resources.data_availability = GasVector { l1_gas: 3, l1_data_gas: 4, l2_gas: 5 };
        }
        if let TransactionReceipt::Invoke(receipt) = &mut blockifier_receipt {
            receipt.actual_fee = FeePayment { amount: Felt::from(20u64), unit: PriceUnit::Wei };
            receipt.execution_resources.steps = 200;
            receipt.execution_resources.memory_holes = 4;
            receipt.execution_resources.data_availability = GasVector { l1_gas: 6, l1_data_gas: 7, l2_gas: 8 };
        }

        let report = compare(&[row(hash, execution_box_receipt)], &[artifact(blockifier_receipt)], &[hash]);

        assert!(report.strict_mismatches.is_empty());
        assert!(report.allowed_mismatches.iter().any(|mismatch| mismatch.category == MismatchCategory::Fee));
        assert!(report.diagnostics.iter().any(|mismatch| mismatch.category == MismatchCategory::Resource));
        assert!(report.diagnostics.iter().any(|mismatch| mismatch.category == MismatchCategory::Commitment));
    }

    #[test]
    fn per_transaction_state_diff_is_diagnostic() {
        let hash = Felt::ONE;
        let mut execution_box = row(hash, receipt(hash));
        execution_box.state_diff.nonces.insert(Felt::ONE, Felt::TWO);
        let blockifier = artifact(receipt(hash));

        let report = compare(&[execution_box], &[blockifier], &[hash]);

        assert!(report.strict_mismatches.is_empty());
        assert!(report.diagnostics.iter().any(|mismatch| mismatch.category == MismatchCategory::StateUpdate));
    }

    fn fee_transfer(sender: Felt, recipient: Felt, low: Felt) -> Event {
        Event { from_address: FEE_TOKEN, keys: vec![TRANSFER_SELECTOR], data: vec![sender, recipient, low, Felt::ZERO] }
    }

    #[test]
    fn block_2479548_fee_transfer_allows_only_amount_limbs() {
        let hash = Felt::ONE;
        let sender = Felt::from(11u64);
        let sequencer = Felt::from(12u64);
        let mut execution_box_receipt = receipt(hash);
        let mut blockifier_receipt = receipt(hash);
        execution_box_receipt.events_mut().push(fee_transfer(sender, sequencer, Felt::from(10u64)));
        blockifier_receipt.events_mut().push(fee_transfer(sender, sequencer, Felt::from(20u64)));

        let report =
            compare(&[row_with_sender(hash, execution_box_receipt, sender)], &[artifact(blockifier_receipt)], &[hash]);

        assert!(report.strict_mismatches.is_empty());
        assert!(report.allowed_mismatches.iter().any(|mismatch| {
            mismatch.category == MismatchCategory::Fee && mismatch.field_path.ends_with("data[2]")
        }));
    }

    #[test]
    fn cairo_one_fee_transfer_allows_only_amount_limbs() {
        let hash = Felt::ONE;
        let sender = Felt::from(11u64);
        let sequencer = Felt::from(12u64);
        let cairo_one_transfer = |amount| Event {
            from_address: FEE_TOKEN,
            keys: vec![TRANSFER_SELECTOR, sender, sequencer],
            data: vec![amount, Felt::ZERO],
        };
        let mut execution_box_receipt = receipt(hash);
        execution_box_receipt.events_mut().push(cairo_one_transfer(Felt::from(10u64)));
        let mut blockifier_receipt = receipt(hash);
        blockifier_receipt.events_mut().push(cairo_one_transfer(Felt::from(20u64)));

        let report =
            compare(&[row_with_sender(hash, execution_box_receipt, sender)], &[artifact(blockifier_receipt)], &[hash]);

        assert!(report.strict_mismatches.is_empty());
        assert!(report.allowed_mismatches.iter().any(|mismatch| {
            mismatch.category == MismatchCategory::Fee && mismatch.field_path.ends_with("data[0]")
        }));
    }

    #[test]
    fn block_2479548_event_regressions_are_strict() {
        let hash_1 = Felt::ONE;
        let hash_2 = Felt::TWO;
        let hash_3 = Felt::THREE;
        let sender = Felt::from(11u64);
        let sequencer = Felt::from(12u64);

        let account_event =
            Event { from_address: sender, keys: vec![TRANSACTION_EXECUTED_SELECTOR, hash_1], data: vec![] };
        let legacy_transfer = fee_transfer(sender, sequencer, Felt::from(20u64));
        let wrong_abi_transfer = Event {
            from_address: FEE_TOKEN,
            keys: vec![TRANSFER_SELECTOR, sender, sequencer],
            data: vec![Felt::from(10u64), Felt::ZERO],
        };
        let source_balance = Event {
            from_address: Felt::from(99u64),
            keys: vec![BALANCE_UPDATE_SELECTOR],
            data: vec![sender, Felt::from(20u64), Felt::from(30u64), Felt::from(40u64), Felt::ZERO],
        };
        let mut wrong_balance = source_balance.clone();
        wrong_balance.data[3] = Felt::from(39u64);

        let execution_box_account_receipt = receipt(hash_1);
        let mut blockifier_account_receipt = receipt(hash_1);
        blockifier_account_receipt.events_mut().push(account_event);
        let mut execution_box_fee_receipt = receipt(hash_2);
        execution_box_fee_receipt.events_mut().push(wrong_abi_transfer);
        let mut blockifier_fee_receipt = receipt(hash_2);
        blockifier_fee_receipt.events_mut().push(legacy_transfer);
        let mut execution_box_balance_receipt = receipt(hash_3);
        execution_box_balance_receipt.events_mut().push(wrong_balance);
        let mut blockifier_balance_receipt = receipt(hash_3);
        blockifier_balance_receipt.events_mut().push(source_balance);

        let report = compare(
            &[
                row(hash_1, execution_box_account_receipt),
                row_with_sender(hash_2, execution_box_fee_receipt, sender),
                row(hash_3, execution_box_balance_receipt),
            ],
            &[
                artifact(blockifier_account_receipt),
                artifact(blockifier_fee_receipt),
                artifact(blockifier_balance_receipt),
            ],
            &[hash_1, hash_2, hash_3],
        );

        assert!(report.strict_mismatches.iter().any(|mismatch| mismatch.field_path == "receipt.events.len"));
        assert!(report.strict_mismatches.iter().any(|mismatch| mismatch.field_path.ends_with("keys")));
        assert!(report.strict_mismatches.iter().any(|mismatch| mismatch.field_path.ends_with("data[3]")));
        assert!(report.strict_mismatches.iter().any(|mismatch| mismatch.category == MismatchCategory::Commitment));
    }

    #[test]
    fn fee_transfer_sender_recipient_and_placement_remain_strict() {
        let hash = Felt::ONE;
        let sender = Felt::from(11u64);
        let sequencer = Felt::from(12u64);
        let compare_fee_events = |execution_box_event, blockifier_event| {
            let mut execution_box_receipt = receipt(hash);
            let mut blockifier_receipt = receipt(hash);
            execution_box_receipt.events_mut().push(execution_box_event);
            blockifier_receipt.events_mut().push(blockifier_event);
            compare(&[row_with_sender(hash, execution_box_receipt, sender)], &[artifact(blockifier_receipt)], &[hash])
        };

        let sender_report = compare_fee_events(
            fee_transfer(Felt::from(13u64), sequencer, Felt::from(10u64)),
            fee_transfer(sender, sequencer, Felt::from(20u64)),
        );
        assert!(sender_report.strict_mismatches.iter().any(|mismatch| mismatch.field_path.ends_with("data[0]")));

        let recipient_report = compare_fee_events(
            fee_transfer(sender, Felt::from(14u64), Felt::from(10u64)),
            fee_transfer(sender, sequencer, Felt::from(20u64)),
        );
        assert!(recipient_report.strict_mismatches.iter().any(|mismatch| mismatch.field_path.ends_with("data[1]")));

        let unrelated_transfer_report = compare_fee_events(
            fee_transfer(sender, Felt::from(15u64), Felt::from(10u64)),
            fee_transfer(sender, Felt::from(15u64), Felt::from(20u64)),
        );
        assert!(unrelated_transfer_report
            .strict_mismatches
            .iter()
            .any(|mismatch| mismatch.field_path.ends_with("data[2]")));

        let other_event = Event { from_address: Felt::ONE, keys: vec![Felt::TWO], data: vec![] };
        let mut execution_box_receipt = receipt(hash);
        execution_box_receipt
            .events_mut()
            .extend([fee_transfer(sender, sequencer, Felt::from(10u64)), other_event.clone()]);
        let mut blockifier_receipt = receipt(hash);
        blockifier_receipt.events_mut().extend([other_event, fee_transfer(sender, sequencer, Felt::from(20u64))]);
        let placement_report =
            compare(&[row_with_sender(hash, execution_box_receipt, sender)], &[artifact(blockifier_receipt)], &[hash]);
        assert!(placement_report
            .strict_mismatches
            .iter()
            .any(|mismatch| mismatch.field_path.ends_with("from_address")));
    }
}
