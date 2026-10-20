//! Dataset ingestion and signed transaction submission on the private Committed Data listener.
//! Imports are durable before transaction admission. Rejected transactions can leave unused datasets.
use crate::{Starknet, StarknetRpcApiError};
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use jsonrpsee::core::{async_trait, RpcResult};
use jsonrpsee::types::ErrorObjectOwned;
use m_proc_macros::versioned_rpc;
use mp_convert::Felt;
use mp_gateway::committed_data::CommittedDataUpdate;
use mp_rpc::v0_10_2::BroadcastedTxn;
use mp_utils::service::MadaraServiceId;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
};

const MAX_SIGNATURE_CHECKS: usize = 4;
const MAX_SIGNATURES: usize = 32;
const MAX_DATASETS_PER_TRANSACTION: usize = 4;
static SIGNATURE_CHECKS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_SIGNATURE_CHECKS)));

#[derive(Debug, thiserror::Error)]
enum CommittedDataRpcError {
    #[error("Unauthorized committed-data import")]
    UnauthorizedImport,
    #[error("Signature verification busy; retry later")]
    SignatureVerificationBusy,
    #[error("Combined submission requires active local block production")]
    LocalProductionRequired,
    #[error("Expected 1..=4 committed-data datasets")]
    InvalidDatasetCount,
    #[error("Query transactions cannot be submitted")]
    QueryTransaction,
}

impl From<CommittedDataRpcError> for ErrorObjectOwned {
    fn from(error: CommittedDataRpcError) -> Self {
        let code = match &error {
            CommittedDataRpcError::UnauthorizedImport => -32001,
            CommittedDataRpcError::SignatureVerificationBusy => -32002,
            CommittedDataRpcError::LocalProductionRequired => -32003,
            CommittedDataRpcError::InvalidDatasetCount | CommittedDataRpcError::QueryTransaction => -32602,
        };
        Self::owned(code, error.to_string(), None::<()>)
    }
}

/// Felt key/value pairs, bounded while decoding rather than after allocation.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommittedDataEntries(Vec<(Felt, Felt)>);

impl CommittedDataEntries {
    /// Consumes the bounded wire value without copying its dataset.
    pub fn into_inner(self) -> Vec<(Felt, Felt)> {
        self.0
    }
}

impl TryFrom<Vec<(Felt, Felt)>> for CommittedDataEntries {
    type Error = blockifier::execution::syscalls::committed_data::CommittedDataError;
    fn try_from(values: Vec<(Felt, Felt)>) -> Result<Self, Self::Error> {
        if values.is_empty() || values.len() > MAX_COMMITTED_DATA_VALUES {
            return Err(Self::Error::InvalidLength);
        }
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for CommittedDataEntries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValuesVisitor;
        impl<'de> serde::de::Visitor<'de> for ValuesVisitor {
            type Value = CommittedDataEntries;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "1..={MAX_COMMITTED_DATA_VALUES} committed-data entries")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<(Felt, Felt)>()? {
                    if values.len() == MAX_COMMITTED_DATA_VALUES {
                        return Err(serde::de::Error::custom("Too many committed-data entries"));
                    }
                    values.push(value);
                }
                if values.is_empty() {
                    return Err(serde::de::Error::custom("Empty committed-data dataset"));
                }
                Ok(CommittedDataEntries(values))
            }
        }
        deserializer.deserialize_seq(ValuesVisitor)
    }
}

/// One allowlisted Stark-curve signer. This authorizes storage, not publication of an Oracle root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSignature {
    pub public_key: Felt,
    pub r: Felt,
    pub s: Felt,
}

/// One independently authenticated, immutable dataset to import before transaction submission.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedDataImport {
    /// If present, entries are upserts relative to this namespace/base; otherwise a full map.
    #[serde(default)]
    pub update: Option<CommittedDataUpdate>,
    pub root: Felt,
    pub entries: CommittedDataEntries,
    #[serde(default)]
    pub signatures: Vec<ImportSignature>,
}

/// At most four datasets are decoded for a single signed transaction.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommittedDataImports(pub Vec<CommittedDataImport>);

impl<'de> Deserialize<'de> for CommittedDataImports {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ImportsVisitor;
        impl<'de> serde::de::Visitor<'de> for ImportsVisitor {
            type Value = CommittedDataImports;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("1..=4 committed-data datasets")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut datasets = Vec::with_capacity(MAX_DATASETS_PER_TRANSACTION);
                loop {
                    if datasets.len() == MAX_DATASETS_PER_TRANSACTION {
                        if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
                            return Err(serde::de::Error::custom("Too many committed-data datasets"));
                        }
                        break;
                    }
                    let Some(dataset) = sequence.next_element::<CommittedDataImport>()? else { break };
                    datasets.push(dataset);
                }
                if datasets.is_empty() {
                    return Err(serde::de::Error::custom("Empty committed-data dataset list"));
                }
                Ok(CommittedDataImports(datasets))
            }
        }
        deserializer.deserialize_seq(ImportsVisitor)
    }
}

/// Durable data and transaction admission. Poll normal receipts for execution and finality.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportAndSubmitResult {
    pub transaction_hash: Felt,
}

/// Canonical signing preimage: ASCII domain || chain-id felt (32 BE) || root (32 BE) || count (4 BE).
/// Starknet Keccak produces a 250-bit message supported by Stark ECDSA. Replay of the same dataset
/// is intentionally idempotent; there is no expiry or off-chain clock dependency.
pub fn import_message_hash(chain_id: Felt, root: Felt, count: u32) -> Felt {
    let mut bytes = b"MADARA_COMMITTED_DATA_IMPORT_V1".to_vec();
    bytes.extend_from_slice(&chain_id.to_bytes_be());
    bytes.extend_from_slice(&root.to_bytes_be());
    bytes.extend_from_slice(&count.to_be_bytes());
    starknet_core::utils::starknet_keccak(&bytes)
}

/// Empty policy disables authorization. Otherwise require 1..=32 distinct, allowed, valid signers.
fn validate_import_signatures(
    allowed: &[Felt],
    chain_id: &starknet_api::core::ChainId,
    root: Felt,
    count: usize,
    signatures: &[ImportSignature],
) -> RpcResult<()> {
    let message = import_message_hash(
        Felt::try_from(chain_id).map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::UnauthorizedImport))?,
        root,
        u32::try_from(count).map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::UnauthorizedImport))?,
    );
    validate_signatures(allowed, message, signatures)
}

/// Update signatures authorize the resulting map in a namespace, not its publication on-chain.
/// The base presence byte distinguishes an empty base from the felt zero root.
pub fn update_message_hash(chain_id: Felt, update: CommittedDataUpdate, root: Felt, count: u32) -> Felt {
    let mut bytes = b"MADARA_COMMITTED_DATA_UPDATE_V1".to_vec();
    for value in [chain_id, update.application_id, update.dataset_id] {
        bytes.extend_from_slice(&value.to_bytes_be());
    }
    bytes.push(u8::from(update.base_root.is_some()));
    bytes.extend_from_slice(&update.base_root.unwrap_or(Felt::ZERO).to_bytes_be());
    bytes.extend_from_slice(&root.to_bytes_be());
    bytes.extend_from_slice(&count.to_be_bytes());
    starknet_core::utils::starknet_keccak(&bytes)
}

fn validate_signatures(allowed: &[Felt], message: Felt, signatures: &[ImportSignature]) -> RpcResult<()> {
    let reject = || CommittedDataRpcError::UnauthorizedImport.into();
    if signatures.len() > MAX_SIGNATURES {
        return Err(reject());
    }
    if allowed.is_empty() {
        return Ok(());
    }
    if signatures.is_empty() {
        return Err(reject());
    }
    let mut seen = HashSet::new();
    for signature in signatures {
        if !allowed.contains(&signature.public_key)
            || !seen.insert(signature.public_key)
            || !starknet_core::crypto::ecdsa_verify(
                &signature.public_key,
                &message,
                &starknet_core::crypto::Signature { r: signature.r, s: signature.s },
            )
            .unwrap_or(false)
        {
            return Err(reject());
        }
    }
    Ok(())
}

fn local_submission_ready(starknet: &Starknet) -> bool {
    starknet.committed_data_transaction_submission
        && starknet.ctx.service_status_actual(MadaraServiceId::BlockProduction).is_on()
        && !starknet.ctx.service_status(MadaraServiceId::L2Sync).is_on()
}

/// Private dataset API. Mount only on the separately enabled committed-data listener.
#[versioned_rpc("V0_1_0", "madara")]
pub trait CommittedDataRpcApi {
    /// Merge upserts into an explicit retained base and check the expected resulting root.
    /// A null base initializes an unused namespace or retries an already registered root.
    /// This method does not update the on-chain root.
    #[method(name = "updateCommittedDataSet")]
    async fn update_committed_data_set(
        &self,
        update: CommittedDataUpdate,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()>;

    /// Imports and authenticates private data; requires available import capacity.
    /// Acknowledges only after durable storage. Does not publish or authorize its root on-chain.
    #[method(name = "importCommittedDataSet")]
    async fn import_committed_data_snapshot(
        &self,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()>;

    /// Durably imports 1..=4 datasets, then submits any signed Starknet account transaction.
    /// A multicall is encoded inside one INVOKE transaction. Dataset imports are not rolled back
    /// if a later import or transaction fails. Returns the transaction hash after admission;
    /// callers poll normal transaction status/receipts for preconfirmed execution and finality.
    #[method(name = "importCommittedDataAndSubmitTransaction")]
    async fn import_committed_data_and_submit_transaction(
        &self,
        datasets: CommittedDataImports,
        transaction: BroadcastedTxn,
    ) -> RpcResult<ImportAndSubmitResult>;
}

#[async_trait]
impl CommittedDataRpcApiV0_1_0Server for Starknet {
    async fn update_committed_data_set(
        &self,
        update: CommittedDataUpdate,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()> {
        let values = entries.into_inner();
        let allowed = Arc::clone(&self.committed_data_signers);
        let chain_id = Felt::try_from(&self.backend.chain_config().chain_id)
            .map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::UnauthorizedImport))?;
        let count = values.len() as u32; // CommittedDataEntries enforces the protocol bound.
        let signatures = signatures.unwrap_or_default();
        let permit = SIGNATURE_CHECKS
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::SignatureVerificationBusy))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            validate_signatures(&allowed, update_message_hash(chain_id, update, root, count), &signatures)
        })
        .await
        .map_err(|error| StarknetRpcApiError::from(anyhow::anyhow!(error)))??;
        self.backend.update_committed_data(update, root, values).await.map_err(StarknetRpcApiError::from)?;
        Ok(())
    }

    async fn import_committed_data_snapshot(
        &self,
        root: Felt,
        entries: CommittedDataEntries,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()> {
        let values = entries.into_inner();
        let allowed = Arc::clone(&self.committed_data_signers);
        let chain_id = self.backend.chain_config().chain_id.clone();
        let count = values.len();
        let signatures = signatures.unwrap_or_default();
        let permit = SIGNATURE_CHECKS
            .clone()
            .try_acquire_owned()
            .map_err(|_| ErrorObjectOwned::from(CommittedDataRpcError::SignatureVerificationBusy))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            validate_import_signatures(&allowed, &chain_id, root, count, &signatures)
        })
        .await
        .map_err(|e| StarknetRpcApiError::from(anyhow::anyhow!(e)))??;
        self.backend.import_committed_data_snapshot(root, values).await.map_err(StarknetRpcApiError::from)?;
        Ok(())
    }

    async fn import_committed_data_and_submit_transaction(
        &self,
        datasets: CommittedDataImports,
        transaction: BroadcastedTxn,
    ) -> RpcResult<ImportAndSubmitResult> {
        if !local_submission_ready(self) {
            return Err(CommittedDataRpcError::LocalProductionRequired.into());
        }
        if !(1..=MAX_DATASETS_PER_TRANSACTION).contains(&datasets.0.len()) {
            return Err(CommittedDataRpcError::InvalidDatasetCount.into());
        }
        // The transaction is deliberately submitted through the same validated path as public RPC.
        // In particular, do not accept a query-only transaction as a publication request.
        if transaction.is_query() {
            return Err(CommittedDataRpcError::QueryTransaction.into());
        }
        for dataset in datasets.0 {
            match dataset.update {
                Some(update) => {
                    self.update_committed_data_set(update, dataset.root, dataset.entries, Some(dataset.signatures))
                        .await?
                }
                None => {
                    self.import_committed_data_snapshot(dataset.root, dataset.entries, Some(dataset.signatures)).await?
                }
            }
        }
        // Catch a sequencer that entered catch-up while the (possibly large) imports were running.
        if !local_submission_ready(self) {
            return Err(CommittedDataRpcError::LocalProductionRequired.into());
        }
        let transaction_hash = match transaction {
            BroadcastedTxn::Invoke(tx) => {
                self.transaction_submitter
                    .submit_invoke_transaction(tx)
                    .await
                    .map_err(StarknetRpcApiError::from)?
                    .transaction_hash
            }
            BroadcastedTxn::Declare(tx) => {
                self.transaction_submitter
                    .submit_declare_transaction(tx)
                    .await
                    .map_err(StarknetRpcApiError::from)?
                    .transaction_hash
            }
            BroadcastedTxn::DeployAccount(tx) => {
                self.transaction_submitter
                    .submit_deploy_account_transaction(tx)
                    .await
                    .map_err(StarknetRpcApiError::from)?
                    .transaction_hash
            }
        };
        Ok(ImportAndSubmitResult { transaction_hash })
    }
}

#[cfg(test)]
mod committed_data_tests {
    use super::*;
    use mc_db::{MadaraBackend, MadaraStorageRead};
    use mc_devnet::{Call, DevnetPredeployedContract, Multicall, Selector};
    use mc_submit_tx::{SubmitTransaction, SubmitTransactionError};
    use mp_convert::ToFelt;
    use mp_rpc::v0_10_2::{
        AddInvokeTransactionResult, BroadcastedDeclareTxn, BroadcastedDeployAccountTxn, BroadcastedInvokeTxn,
        ClassAndTxnHash, ContractAndTxnHash,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Account multicalls, including Oracle root + final-price calls, are one signed INVOKE.
    /// Use two fee-token calls here so the RPC routing test does not depend on an Oracle fixture.
    fn signed_two_call_tx(backend: &MadaraBackend, account: &DevnetPredeployedContract) -> BroadcastedTxn {
        use mp_rpc::v0_9_0::{BroadcastedInvokeTxn as OldInvoke, BroadcastedTxn as OldTxn};
        use mp_transactions::{validated::TxTimestamp, IntoStarknetApiExt};
        let OldTxn::Invoke(OldInvoke::V3(mut tx)) =
            crate::test_utils::devnet_transfer_tx(backend, account, Felt::ZERO, false)
        else {
            unreachable!("devnet transfer helper always constructs an invoke v3")
        };
        let call = || Call {
            to: backend.chain_config().native_fee_token_address.to_felt(),
            selector: Selector::from("transfer"),
            calldata: vec![account.address, Felt::ONE, Felt::ZERO],
        };
        tx.calldata = Multicall::default().with(call()).with(call()).flatten().collect::<Vec<_>>().into();
        let hash = OldTxn::Invoke(OldInvoke::V3(tx.clone()))
            .into_validated_tx(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
                TxTimestamp::now(),
            )
            .unwrap()
            .hash;
        let signature = account.secret.sign(&hash).unwrap();
        tx.signature = vec![signature.r, signature.s].into();
        serde_json::from_value(serde_json::to_value(OldTxn::Invoke(OldInvoke::V3(tx))).unwrap()).unwrap()
    }

    struct RecordingSubmitter {
        attempts: AtomicUsize,
        reject: bool,
    }

    #[async_trait]
    impl SubmitTransaction for RecordingSubmitter {
        async fn submit_declare_transaction(
            &self,
            _tx: BroadcastedDeclareTxn,
        ) -> Result<ClassAndTxnHash, SubmitTransactionError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_deploy_account_transaction(
            &self,
            _tx: BroadcastedDeployAccountTxn,
        ) -> Result<ContractAndTxnHash, SubmitTransactionError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(SubmitTransactionError::Unsupported)
        }

        async fn submit_invoke_transaction(
            &self,
            _tx: BroadcastedInvokeTxn,
        ) -> Result<AddInvokeTransactionResult, SubmitTransactionError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if self.reject {
                Err(SubmitTransactionError::Unsupported)
            } else {
                Ok(AddInvokeTransactionResult { transaction_hash: Felt::THREE })
            }
        }
    }

    #[tokio::test]
    async fn combined_rpc_imports_before_invoke_and_does_not_submit_after_import_failure() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        let (backend, mut starknet, keys) = crate::test_utils::rpc_test_setup_with_execution().await;
        let submitter = Arc::new(RecordingSubmitter { attempts: AtomicUsize::new(0), reject: false });
        starknet.transaction_submitter = submitter.clone();
        let account = keys.0.first().unwrap();
        let tx = signed_two_call_tx(&backend, account);
        let values: Vec<_> = vec![(Felt::ZERO, Felt::from(42_u32))];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        let funding_values: Vec<_> = vec![(Felt::ZERO, Felt::from(43_u32))];
        let funding_root = CommittedDataSet::new(funding_values.clone()).unwrap().root();

        let call = |funding_root, tx: &BroadcastedTxn| {
            serde_json::json!({"jsonrpc":"2.0","id":1,
                "method":"madara_V0_1_0_importCommittedDataAndSubmitTransaction",
                "params":[[{"root":root,"entries":values,
                            "update":{"application_id":"0x1","dataset_id":"0x2","base_root":null}},
                           {"root":funding_root,"entries":funding_values,
                            "update":{"application_id":"0x1","dataset_id":"0x2","base_root":root}}],tx]})
            .to_string()
        };
        let module = crate::rpc_api_committed_data(&starknet).unwrap();
        let (response, _) = module.raw_json_request(&call(funding_root, &tx), 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["error"]["code"], -32003);
        assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().is_none());
        starknet.committed_data_transaction_submission = true;
        let module = crate::rpc_api_committed_data(&starknet).unwrap();
        let (response, _) = module.raw_json_request(&call(funding_root, &tx), 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["error"]["code"], -32003);
        assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().is_none());
        starknet.ctx.service_remove(MadaraServiceId::L2Sync);
        let (response, _) = module.raw_json_request(&call(Felt::ONE, &tx), 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_some(), "invalid second root must fail: {response}");
        assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
        assert!(backend.committed_data_witness(funding_root, Felt::ZERO).unwrap().is_none());
        assert_eq!(submitter.attempts.load(Ordering::SeqCst), 0);

        let (response, _) = module.raw_json_request(&call(funding_root, &tx), 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["result"]["transaction_hash"], serde_json::json!(Felt::THREE));
        assert_eq!(response["result"], serde_json::json!({"transaction_hash": Felt::THREE}));
        // No receipt was inserted: admission must return without waiting for a block.
        assert!(backend.view_on_latest().find_transaction_by_hash(&Felt::THREE).unwrap().is_none());
        assert_eq!(submitter.attempts.load(Ordering::SeqCst), 1);
        assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
        assert!(backend.committed_data_witness(funding_root, Felt::ZERO).unwrap().unwrap().verify());
    }

    #[tokio::test]
    async fn rejected_submission_leaves_only_an_unused_durable_dataset() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        let (backend, mut starknet, keys) = crate::test_utils::rpc_test_setup_with_execution().await;
        starknet.ctx.service_remove(MadaraServiceId::L2Sync);
        starknet.committed_data_transaction_submission = true;
        let submitter = Arc::new(RecordingSubmitter { attempts: AtomicUsize::new(0), reject: true });
        starknet.transaction_submitter = submitter.clone();
        let tx = signed_two_call_tx(&backend, keys.0.first().unwrap());
        let values: Vec<_> = vec![(Felt::ZERO, Felt::from(99_u32))];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        let module = crate::rpc_api_committed_data(&starknet).unwrap();
        let request = serde_json::json!({"jsonrpc":"2.0","id":1,
            "method":"madara_V0_1_0_importCommittedDataAndSubmitTransaction",
            "params":[[{"root":root,"entries":values}],tx]})
        .to_string();
        let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_some(), "submission must fail: {response}");
        assert_eq!(submitter.attempts.load(Ordering::SeqCst), 1);
        assert!(backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
    }

    #[tokio::test]
    async fn dedicated_module_only_imports_without_admin_or_read_exposure() {
        use crate::{rpc_api_admin, rpc_api_committed_data, rpc_api_user, test_utils::rpc_test_setup};
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        let (_, mut starknet) = rpc_test_setup();
        let module = rpc_api_committed_data(&starknet).unwrap();
        let mut names: Vec<_> = module.method_names().collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "madara_V0_1_0_importCommittedDataAndSubmitTransaction",
                "madara_V0_1_0_importCommittedDataSet",
                "madara_V0_1_0_updateCommittedDataSet"
            ]
        );
        starknet.set_rpc_unsafe_enabled(true);
        for api in [rpc_api_user(&starknet).unwrap(), rpc_api_admin(&starknet).unwrap()] {
            for name in &names {
                let req = serde_json::json!({"jsonrpc":"2.0", "id":1,"method":name,"params":[]}).to_string();
                let (response, _) = api.raw_json_request(&req, 1).await.unwrap();
                let response: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(response["error"]["code"], -32601);
            }
        }
        let values: Vec<_> = vec![(Felt::ZERO, Felt::from(2000_u32)), (Felt::ONE, Felt::from(3000_u32))];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"madara_V0_1_0_importCommittedDataSet","params":[root,values]}).to_string();
        let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
        let req = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"madara_V0_1_0_getCommittedDataWitness","params":[root,0]}).to_string();
        let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["error"]["code"], -32601);
        let req = serde_json::json!({"jsonrpc":"2.0","id":3,"method":"madara_V0_1_0_importCommittedDataSet","params":[Felt::ONE,values]}).to_string();
        let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_some(), "Root mismatch must not be acknowledged");
    }

    #[tokio::test]
    async fn committed_data_rpc_rejects_unsigned_before_storage_and_accepts_allowed_signature() {
        let (_, mut starknet) = crate::test_utils::rpc_test_setup();
        let public_key = starknet_types_core::curve::AffinePoint::generator().x();
        starknet.set_committed_data_signers(vec![public_key]).unwrap();
        let module = crate::rpc_api_committed_data(&starknet).unwrap();
        let values: Vec<_> = vec![(Felt::ZERO, Felt::ONE)];
        let root =
            blockifier::execution::syscalls::committed_data::CommittedDataSet::new(values.clone()).unwrap().root();
        let call = |params| {
            serde_json::json!({"jsonrpc":"2.0","id":1,
            "method":"madara_V0_1_0_importCommittedDataSet","params":params})
            .to_string()
        };
        let (response, _) = module.raw_json_request(&call(serde_json::json!([root, values])), 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["error"]["code"], -32001);
        assert!(starknet.backend.committed_data_witness(root, Felt::ZERO).unwrap().is_none());
        let message = import_message_hash(Felt::try_from(&starknet.backend.chain_config().chain_id).unwrap(), root, 1);
        let signature = starknet_core::crypto::ecdsa_sign(&Felt::ONE, &message).unwrap();
        let (response, _) = module
            .raw_json_request(
                &call(serde_json::json!([
                    root,
                    values,
                    [ImportSignature { public_key, r: signature.r, s: signature.s }]
                ])),
                1,
            )
            .await
            .unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
        assert!(response.get("error").is_none());
        assert!(starknet.backend.committed_data_witness(root, Felt::ZERO).unwrap().unwrap().verify());
    }

    #[test]
    fn committed_data_authorization_binds_chain_root_count_and_distinct_allowed_signers() {
        use starknet_api::core::ChainId;
        let chain = ChainId::Sepolia;
        let public_key = starknet_types_core::curve::AffinePoint::generator().x();
        let root = Felt::TWO;
        let signature = starknet_core::crypto::ecdsa_sign(
            &Felt::ONE,
            &import_message_hash(Felt::try_from(&chain).unwrap(), root, 3),
        )
        .unwrap();
        let signed = ImportSignature { public_key, r: signature.r, s: signature.s };
        assert!(validate_import_signatures(&[public_key], &chain, root, 3, std::slice::from_ref(&signed)).is_ok());
        assert!(validate_import_signatures(&[], &chain, root, 3, &[]).is_ok());
        assert!(validate_import_signatures(&[public_key], &chain, root, 3, &[]).is_err());
        assert!(validate_import_signatures(&[Felt::ZERO], &chain, root, 3, std::slice::from_ref(&signed)).is_err());
        assert!(validate_import_signatures(&[public_key], &ChainId::Mainnet, root, 3, std::slice::from_ref(&signed))
            .is_err());
        assert!(validate_import_signatures(&[public_key], &chain, Felt::ONE, 3, std::slice::from_ref(&signed)).is_err());
        assert!(validate_import_signatures(&[public_key], &chain, root, 4, std::slice::from_ref(&signed)).is_err());
        assert!(validate_import_signatures(&[public_key], &chain, root, 3, &[signed.clone(), signed.clone()]).is_err());
        assert!(validate_import_signatures(
            &[public_key],
            &chain,
            root,
            3,
            &[ImportSignature { s: Felt::ZERO, ..signed }]
        )
        .is_err());
    }

    #[tokio::test]
    async fn committed_data_update_rpc_authenticates_namespace_base_and_expected_root() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        // Independent Python signing vector for the initial two-market map.
        let fixture = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
        assert_eq!(
            update_message_hash(
                Felt::from_hex_unchecked("0x534e5f5345504f4c4941"),
                fixture,
                Felt::from_hex_unchecked("0x5a32e2ffd847d637b53eff4f469dcea2ef05175e9b5497282eb9bfbbe14ecda"),
                2
            ),
            Felt::from_hex_unchecked("0x1229be96497ad1baa92f6d7154f9979c23cb11bb31b2620a2715bd650b4ac9d")
        );
        assert!(serde_json::from_value::<CommittedDataUpdate>(serde_json::json!({
            "application_id":"0x1","dataset_id":"0x2"}))
        .is_err());
        let (backend, mut starknet) = crate::test_utils::rpc_test_setup();
        let public_key = starknet_types_core::curve::AffinePoint::generator().x();
        starknet.set_committed_data_signers(vec![public_key]).unwrap();
        let context = CommittedDataUpdate { application_id: Felt::ONE, dataset_id: Felt::TWO, base_root: None };
        let entries = vec![(Felt::MAX, Felt::THREE)];
        let root = CommittedDataSet::new(entries.clone()).unwrap().root();
        let chain = Felt::try_from(&backend.chain_config().chain_id).unwrap();
        let message = update_message_hash(chain, context, root, 1);
        for other in [
            update_message_hash(chain + Felt::ONE, context, root, 1),
            update_message_hash(chain, CommittedDataUpdate { application_id: Felt::TWO, ..context }, root, 1),
            update_message_hash(chain, CommittedDataUpdate { dataset_id: Felt::THREE, ..context }, root, 1),
            update_message_hash(chain, CommittedDataUpdate { base_root: Some(Felt::ZERO), ..context }, root, 1),
            update_message_hash(chain, context, root + Felt::ONE, 1),
            update_message_hash(chain, context, root, 2),
            import_message_hash(chain, root, 1),
        ] {
            assert_ne!(message, other);
        }
        let sig = starknet_core::crypto::ecdsa_sign(&Felt::ONE, &message).unwrap();
        let signature = ImportSignature { public_key, r: sig.r, s: sig.s };
        let module = crate::rpc_api_committed_data(&starknet).unwrap();
        let call = |context: CommittedDataUpdate, signatures: Vec<ImportSignature>| {
            serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"madara_V0_1_0_updateCommittedDataSet",
            "params":[context,root,entries,signatures]})
            .to_string()
        };
        for request in [
            call(context, vec![]),
            call(CommittedDataUpdate { dataset_id: Felt::THREE, ..context }, vec![signature.clone()]),
        ] {
            let (response, _) = module.raw_json_request(&request, 1).await.unwrap();
            assert_eq!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["error"]["code"], -32001);
            assert!(backend.db.get_committed_data_count(root).unwrap().is_none());
        }
        let (response, _) = module.raw_json_request(&call(context, vec![signature]), 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(backend.db.list_committed_data_versions(None).unwrap().len(), 1);
    }

    #[test]
    fn committed_data_import_array_is_bounded_during_decoding() {
        assert!(serde_json::from_str::<CommittedDataEntries>("[]").is_err());
        let encoded = serde_json::to_string(&vec![(Felt::MAX, Felt::ZERO)]).unwrap();
        let entries: CommittedDataEntries = serde_json::from_str(&encoded).unwrap();
        assert_eq!(entries.0, vec![(Felt::MAX, Felt::ZERO)]);
        let count = blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES + 1;
        let encoded = format!("[{}]", std::iter::repeat_n("[\"0x0\",\"0x0\"]", count).collect::<Vec<_>>().join(","));
        assert!(serde_json::from_str::<CommittedDataEntries>(&encoded).is_err());
        let item = serde_json::json!({"root":Felt::ONE,"entries":[[Felt::ZERO,Felt::ONE]]});
        assert!(serde_json::from_value::<CommittedDataImports>(serde_json::json!([])).is_err());
        assert!(serde_json::from_value::<CommittedDataImports>(serde_json::json!(vec![item.clone(); 4])).is_ok());
        assert!(serde_json::from_value::<CommittedDataImports>(serde_json::json!(vec![item; 5])).is_err());
    }
}
