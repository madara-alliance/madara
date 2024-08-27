//! Step 1. pre-validate: [`UnverifiedFullBlock`] ====[`crate::pre_validate`]===> [`PreValidatedBlock`]
//! Step 2. verify_apply: [`PreValidatedBlock`] ====[`crate::verify_apply`]===> [`BlockImportResult`]

use std::mem;

use bitvec::vec::BitVec;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use starknet_api::core::ChainId;
use starknet_core::types::Felt;

use dp_block::{
    header::{GasPrices, L1DataAvailabilityMode},
    Header,
};
use dp_chain_config::StarknetVersion;
use dp_class::{ClassInfo, ContractClass, ConvertedClass, ToCompiledClass};
use dp_convert::ToFelt;
use dp_receipt::TransactionReceipt;
use dp_state_update::StateDiff;
use dp_transactions::Transaction;
use dp_validation::{PreValidate, ValidationContext};
use starknet_types_core::hash::{Pedersen, Poseidon, StarkHash};

use crate::BlockImportError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnverifiedHeader {
    /// The hash of this block’s parent. When set to None, it will be deduced from the latest block in storage.
    pub parent_block_hash: Option<Felt>,
    /// The Starknet address of the sequencer that created this block.
    pub sequencer_address: Felt,
    /// The time the sequencer created this block before executing transactions
    pub block_timestamp: u64,
    /// The version of the Starknet protocol used when creating this block
    pub protocol_version: StarknetVersion,
    /// Gas prices for this block
    pub l1_gas_price: GasPrices,
    /// The mode of data availability for this block
    pub l1_da_mode: L1DataAvailabilityMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredClass {
    pub class_hash: Felt,
    pub contract_class: ContractClass,
    pub compiled_class_hash: Felt,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct UnverifiedCommitments {
    pub transaction_count: Option<u64>,
    pub transaction_commitment: Option<Felt>,
    pub event_count: Option<u64>,
    pub event_commitment: Option<Felt>,
    pub state_diff_length: Option<u64>,
    pub state_diff_commitment: Option<Felt>,
    pub receipt_commitment: Option<Felt>,
    /// Global state root
    pub global_state_root: Option<Felt>,
    /// Expected block hash
    pub block_hash: Option<Felt>,
}

/// An unverified pending full block as input for the block import pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnverifiedPendingFullBlock {
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
}

impl PreValidate for UnverifiedPendingFullBlock {
    type Output = PreValidatedPendingBlock;
    type ValidationError = BlockImportError;

    fn pre_validate(mut self, context: &ValidationContext) -> Result<Self::Output, Self::ValidationError> {
        let classes = mem::take(&mut self.declared_classes);

        let converted_classes = convert_classes(classes, context)?;
        let _tx_hashes = transaction_hashes(&self.receipts, &self.transactions, context, None)?;

        Ok(PreValidatedPendingBlock {
            header: self.header,
            transactions: self.transactions,
            state_diff: self.state_diff,
            receipts: self.receipts,
            converted_classes,
        })
    }
}

/// An unverified full block as input for the block import pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnverifiedFullBlock {
    /// When set to None, it will be deduced from the latest block in storage.
    pub unverified_block_number: Option<u64>,
    pub header: UnverifiedHeader,
    pub state_diff: StateDiff,
    pub transactions: Vec<Transaction>,
    pub receipts: Vec<TransactionReceipt>,
    pub declared_classes: Vec<DeclaredClass>,
    // In authority mode, all commitments will be None
    pub commitments: UnverifiedCommitments,
}

impl UnverifiedFullBlock {
    fn receipt_commitment(&self, _context: &ValidationContext) -> Result<Felt, BlockImportError> {
        let hashes = self.receipts.par_iter().map(TransactionReceipt::compute_hash).collect::<Vec<_>>();
        let got = compute_merkle_root::<Poseidon>(&hashes);

        if let Some(expected) = self.commitments.receipt_commitment {
            if expected != got {
                return Err(BlockImportError::ReceiptCommitment { got, expected });
            }
        }
        Ok(got)
    }

    fn state_diff_commitment(&self, _context: &ValidationContext) -> Result<Felt, BlockImportError> {
        let got = self.state_diff.len() as u64;
        if let Some(expected) = self.commitments.state_diff_length {
            if expected != got {
                return Err(BlockImportError::StateDiffLength { got, expected });
            }
        }

        let got = self.state_diff.compute_hash();
        if let Some(expected) = self.commitments.state_diff_commitment {
            if expected != got {
                return Err(BlockImportError::StateDiffCommitment { got, expected });
            }
        }
        Ok(got)
    }

    /// Compute the transaction commitment for a block.
    fn transaction_commitment(&self, context: &ValidationContext) -> Result<Felt, BlockImportError> {
        let starknet_version = self.header.protocol_version;

        let transaction_hashes =
            transaction_hashes(&self.receipts, &self.transactions, context, self.unverified_block_number)?;

        if let Some(expected) = self.commitments.transaction_count {
            if expected != self.transactions.len() as u64 {
                return Err(BlockImportError::TransactionCount { got: self.transactions.len() as _, expected });
            }
        }

        // Compute transaction hashes
        let tx_hashes_with_signature: Vec<_> = self
            .transactions
            .par_iter()
            .zip(transaction_hashes)
            .map(|(tx, tx_hash)| tx.compute_hash_with_signature(tx_hash, starknet_version))
            .collect();

        // Transaction commitment
        let got = if starknet_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
            compute_merkle_root::<Pedersen>(&tx_hashes_with_signature)
        } else {
            compute_merkle_root::<Poseidon>(&tx_hashes_with_signature)
        };

        if let Some(expected) = self.commitments.transaction_commitment.filter(|expected| *expected != got) {
            return Err(BlockImportError::TransactionCommitment { got, expected });
        }

        Ok(got)
    }

    fn event_commitment(&self, _context: &ValidationContext) -> Result<Felt, BlockImportError> {
        let events_with_tx_hash: Vec<_> = self
            .receipts
            .iter()
            .flat_map(|receipt| receipt.events().iter().map(move |event| (receipt.transaction_hash(), event.clone())))
            .collect();

        if let Some(expected) = self.commitments.event_count {
            if expected != events_with_tx_hash.len() as u64 {
                return Err(BlockImportError::EventCount { got: events_with_tx_hash.len() as _, expected });
            }
        }

        let got = if events_with_tx_hash.is_empty() {
            Felt::ZERO
        } else if self.header.protocol_version < StarknetVersion::STARKNET_VERSION_0_13_2 {
            let events_hash =
                events_with_tx_hash.into_par_iter().map(|(_, event)| event.compute_hash_pedersen()).collect::<Vec<_>>();
            compute_merkle_root::<Pedersen>(&events_hash)
        } else {
            let events_hash = events_with_tx_hash
                .into_par_iter()
                .map(|(hash, event)| event.compute_hash_poseidon(&hash))
                .collect::<Vec<_>>();
            compute_merkle_root::<Poseidon>(&events_hash)
        };

        if let Some(expected) = self.commitments.event_commitment {
            if expected != got {
                return Err(BlockImportError::EventCommitment { got, expected });
            }
        }

        Ok(got)
    }

    /// Returns [`ValidatedCommitments`] from the block unverified commitments.
    pub fn valid_commitments(&self, context: &ValidationContext) -> Result<ValidatedCommitments, BlockImportError> {
        let (mut receipt_c, mut state_diff_c, mut transaction_c, mut event_c) = Default::default();
        [
            Box::new(|| {
                receipt_c = self.receipt_commitment(context)?;
                Ok(())
            }) as Box<dyn FnOnce() -> Result<(), BlockImportError> + Send>,
            Box::new(|| {
                state_diff_c = self.state_diff_commitment(context)?;
                Ok(())
            }),
            Box::new(|| {
                transaction_c = self.transaction_commitment(context)?;
                Ok(())
            }),
            Box::new(|| {
                event_c = self.event_commitment(context)?;
                Ok(())
            }),
        ]
        .into_par_iter()
        .map(|f| f())
        .collect::<Result<(), _>>()?;

        Ok(ValidatedCommitments {
            transaction_count: self.transactions.len() as _,
            transaction_commitment: transaction_c,
            event_count: self.receipts.iter().map(|r| r.events().len()).sum::<usize>() as _,
            event_commitment: event_c,
            state_diff_length: self.state_diff.len() as _,
            state_diff_commitment: state_diff_c,
            receipt_commitment: receipt_c,
        })
    }
}

impl PreValidate for UnverifiedFullBlock {
    type Output = PreValidatedBlock;
    type ValidationError = BlockImportError;

    fn pre_validate(mut self, context: &ValidationContext) -> Result<Self::Output, Self::ValidationError> {
        let classes = mem::take(&mut self.declared_classes);

        // unfortunately this is ugly but rayon::join does not have the fast error short circuiting behavior that
        // collecting into a Result has.
        // little known fact this uses the impl FromIterator for () from std, nice trick
        let (mut commitments, mut converted_classes) = Default::default();
        [
            Box::new(|| {
                commitments = self.valid_commitments(context)?;
                Ok(())
            }) as Box<dyn FnOnce() -> Result<(), BlockImportError> + Send>,
            Box::new(|| {
                converted_classes = convert_classes(classes, context)?;
                Ok(())
            }),
        ]
        .into_par_iter()
        .map(|f| f())
        .collect::<Result<(), _>>()?;

        Ok(PreValidatedBlock {
            header: self.header,
            transactions: self.transactions,
            state_diff: self.state_diff,
            receipts: self.receipts,
            commitments,
            converted_classes,
            unverified_global_state_root: self.commitments.global_state_root,
            unverified_block_hash: self.commitments.block_hash,
            unverified_block_number: self.unverified_block_number,
        })
    }
}

// Pre-validate outputs.

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ValidatedCommitments {
    pub transaction_count: u64,
    pub transaction_commitment: Felt,
    pub event_count: u64,
    pub event_commitment: Felt,
    pub state_diff_length: u64,
    pub state_diff_commitment: Felt,
    pub receipt_commitment: Felt,
}

/// Output of the [`crate::pre_validate`] step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreValidatedBlock {
    pub header: UnverifiedHeader,
    pub transactions: Vec<Transaction>,
    pub state_diff: StateDiff,
    pub receipts: Vec<TransactionReceipt>,
    pub commitments: ValidatedCommitments,
    pub converted_classes: Vec<ConvertedClass>,

    pub unverified_global_state_root: Option<Felt>,
    pub unverified_block_hash: Option<Felt>,
    pub unverified_block_number: Option<u64>,
}

/// Output of the [`crate::pre_validate`] step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreValidatedPendingBlock {
    pub header: UnverifiedHeader,
    pub transactions: Vec<Transaction>,
    pub state_diff: StateDiff,
    pub receipts: Vec<TransactionReceipt>,
    pub converted_classes: Vec<ConvertedClass>,
}

// Verify-apply output.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockImportResult {
    pub header: Header,
    pub block_hash: Felt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingBlockImportResult {}

fn transaction_hashes(
    receipts: &[TransactionReceipt],
    transactions: &[Transaction],
    context: &ValidationContext,
    block_n: Option<u64>,
) -> Result<Vec<Felt>, BlockImportError> {
    if receipts.len() != transactions.len() {
        return Err(BlockImportError::TransactionEqualReceiptCount {
            receipts: receipts.len(),
            transactions: transactions.len(),
        });
    }

    // mismatched block hash is allowed for blocks 1466..=2242 on mainnet
    let is_special_trusted_case =
        context.chain_id == ChainId::Mainnet && block_n.is_some_and(|n| (1466..=2242).contains(&n));

    if is_special_trusted_case || context.trust_transaction_hashes {
        Ok(receipts.iter().map(|r| r.transaction_hash()).collect())
    } else {
        transactions
            .par_iter()
            .enumerate()
            .map(|(index, tx)| {
                // Panic safety: receipt count was checked earlier
                let got = receipts[index].transaction_hash();
                let expected = tx.compute_hash(context.chain_id.to_felt(), false, block_n);
                if got != expected {
                    return Err(BlockImportError::TransactionHash { index, got, expected });
                }
                Ok(got)
            })
            .collect()
    }
}

fn convert_classes(
    declared_classes: Vec<DeclaredClass>,
    context: &ValidationContext,
) -> Result<Vec<ConvertedClass>, BlockImportError> {
    declared_classes.into_par_iter().map(|class| class_conversion(class, context)).collect()
}

fn class_conversion(class: DeclaredClass, _context: &ValidationContext) -> Result<ConvertedClass, BlockImportError> {
    let DeclaredClass { class_hash, contract_class, compiled_class_hash } = class;

    // TODO(class_hash, #212): uncomment this when the class hashes are computed correctly accross the entire state
    // let expected =
    //     contract_class.class_hash().map_err(|e| BlockImportError::ComputeClassHashError(e.to_string()))?;
    // if class_hash != expected {
    // }

    let compiled_class = contract_class
        .compile()
        .map_err(|e| BlockImportError::CompilationClassError { error: e.to_string(), class_hash })?;

    let class_info = ClassInfo { contract_class, compiled_class_hash };

    Ok(ConvertedClass { class_infos: (class_hash, class_info), class_compiled: (class_hash, compiled_class) })
}

/// Compute the root hash of a list of values.
// The `HashMapDb` can't fail, so we can safely unwrap the results.
//
// perf: Note that committing changes still has the greatest performance hit
// as this is where the root hash is calculated. Due to the Merkle structure
// of Bonsai Tries, this results in a trie size that grows very rapidly with
// each new insertion. It seems that the only vector of optimization here
// would be to parallelize the tree traversal on insertion and optimize hash computation.
// It seems lambdaclass' crypto lib does not do simd hashing, we may want to look into that.
fn compute_merkle_root<H: StarkHash + Send + Sync>(values: &[Felt]) -> Felt {
    //TODO: replace the identifier by an empty slice when bonsai supports it
    const IDENTIFIER: &[u8] = b"0xinmemory";
    let config = bonsai_trie::BonsaiStorageConfig::default();
    let bonsai_db = bonsai_trie::databases::HashMapDb::<bonsai_trie::id::BasicId>::default();
    let mut bonsai_storage =
        bonsai_trie::BonsaiStorage::<_, _, H>::new(bonsai_db, config).expect("Failed to create bonsai storage");

    values.iter().enumerate().for_each(|(id, value)| {
        let key = BitVec::from_vec(id.to_be_bytes().to_vec());
        bonsai_storage.insert(IDENTIFIER, key.as_bitslice(), value).expect("Failed to insert into bonsai storage");
    });

    let id = bonsai_trie::id::BasicIdBuilder::new().new_id();

    bonsai_storage.commit(id).expect("Failed to commit to bonsai storage");
    bonsai_storage.root_hash(IDENTIFIER).expect("Failed to get root hash")
}

#[cfg(test)]
mod tests {
    use starknet_types_core::hash::Poseidon;

    use super::*;

    #[test]
    fn test_compute_root() {
        let values = vec![Felt::ONE, Felt::TWO, Felt::THREE];
        let root = compute_merkle_root::<Poseidon>(&values);

        assert_eq!(root, Felt::from_hex_unchecked("0x3b5cc7f1292eb3847c3f902d048a7e5dc7702d1c191ccd17c2d33f797e6fc32"));
    }
}
