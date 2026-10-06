//! Dataset ingestion for the opt-in Committed Data RPC listener.
//! Dataset import is durable before success; on-chain publication remains a separate transaction.
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use m_proc_macros::versioned_rpc;
use mp_convert::Felt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
};

static SIGNATURE_CHECKS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

/// Wire-compatible felt array, bounded while decoding rather than after allocation.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct CommittedDataValues(Vec<Felt>);

impl CommittedDataValues {
    /// Consumes the bounded wire value without copying its dataset.
    pub fn into_inner(self) -> Vec<Felt> {
        self.0
    }
}

impl TryFrom<Vec<Felt>> for CommittedDataValues {
    type Error = blockifier::execution::syscalls::committed_data::CommittedDataError;
    fn try_from(values: Vec<Felt>) -> Result<Self, Self::Error> {
        if values.is_empty()
            || values.len() > blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES
        {
            return Err(Self::Error::InvalidLength);
        }
        Ok(Self(values))
    }
}

impl<'de> Deserialize<'de> for CommittedDataValues {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValuesVisitor;
        impl<'de> serde::de::Visitor<'de> for ValuesVisitor {
            type Value = CommittedDataValues;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("1..=524288 committed-data values")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<Felt>()? {
                    if values.len() == blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES {
                        return Err(serde::de::Error::custom("Too many committed-data values"));
                    }
                    values.push(value);
                }
                if values.is_empty() {
                    return Err(serde::de::Error::custom("Empty committed-data dataset"));
                }
                Ok(CommittedDataValues(values))
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
    let reject = || jsonrpsee::types::ErrorObjectOwned::owned(-32001, "Unauthorized committed-data import", None::<()>);
    if signatures.len() > 32 {
        return Err(reject());
    }
    if allowed.is_empty() {
        return Ok(());
    }
    if signatures.is_empty() {
        return Err(reject());
    }
    let message = import_message_hash(
        Felt::try_from(chain_id).map_err(|_| reject())?,
        root,
        u32::try_from(count).map_err(|_| reject())?,
    );
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

/// Private dataset API. Mount only on the separately enabled committed-data listener.
#[versioned_rpc("V0_1_0", "madara")]
pub trait CommittedDataRpcApi {
    /// Imports and authenticates private data; requires available import capacity.
    /// Acknowledges only after durable storage. Does not publish or authorize its root on-chain.
    #[method(name = "importCommittedDataSet")]
    async fn import_committed_data_snapshot(
        &self,
        root: Felt,
        values: CommittedDataValues,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()>;
}

#[async_trait]
impl CommittedDataRpcApiV0_1_0Server for Starknet {
    async fn import_committed_data_snapshot(
        &self,
        root: Felt,
        values: CommittedDataValues,
        signatures: Option<Vec<ImportSignature>>,
    ) -> RpcResult<()> {
        let values = values.into_inner();
        let allowed = Arc::clone(&self.committed_data_signers);
        let chain_id = self.backend.chain_config().chain_id.clone();
        let count = values.len();
        let signatures = signatures.unwrap_or_default();
        let permit = SIGNATURE_CHECKS.clone().try_acquire_owned().map_err(|_| {
            jsonrpsee::types::ErrorObjectOwned::owned(-32002, "Signature verification busy; retry later", None::<()>)
        })?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            validate_import_signatures(&allowed, &chain_id, root, count, &signatures)
        })
        .await
        .map_err(|e| StarknetRpcApiError::from(anyhow::anyhow!(e)))??;
        self.backend.import_committed_data_snapshot(root, values).await.map_err(StarknetRpcApiError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod committed_data_tests {
    use super::*;

    #[tokio::test]
    async fn dedicated_module_only_imports_without_admin_or_read_exposure() {
        use crate::{rpc_api_admin, rpc_api_committed_data, rpc_api_user, test_utils::rpc_test_setup};
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        let (_, mut starknet) = rpc_test_setup();
        let module = rpc_api_committed_data(&starknet).unwrap();
        let mut names: Vec<_> = module.method_names().collect();
        names.sort_unstable();
        assert_eq!(names, vec!["madara_V0_1_0_importCommittedDataSet"]);
        starknet.set_rpc_unsafe_enabled(true);
        for api in [rpc_api_user(&starknet).unwrap(), rpc_api_admin(&starknet).unwrap()] {
            for name in &names {
                let req = serde_json::json!({"jsonrpc":"2.0", "id":1,"method":name,"params":[]}).to_string();
                let (response, _) = api.raw_json_request(&req, 1).await.unwrap();
                let response: serde_json::Value = serde_json::from_str(&response).unwrap();
                assert_eq!(response["error"]["code"], -32601);
            }
        }
        let values = vec![Felt::from(2000_u32), Felt::from(3000_u32)];
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        let req = serde_json::json!({"jsonrpc":"2.0","id":1,"method":names[0],"params":[root,values]}).to_string();
        let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response.get("result"), Some(&serde_json::Value::Null));
        let req = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"madara_V0_1_0_getCommittedDataWitness","params":[root,0]}).to_string();
        let (response, _) = module.raw_json_request(&req, 1).await.unwrap();
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(response["error"]["code"], -32601);
        let req = serde_json::json!({"jsonrpc":"2.0","id":3,"method":names[0],"params":[Felt::ONE,values]}).to_string();
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
        let values = vec![Felt::ONE];
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
        assert!(starknet.backend.committed_data_witness(root, 0).unwrap().is_none());
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
        assert!(starknet.backend.committed_data_witness(root, 0).unwrap().unwrap().verify());
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
        assert!(validate_import_signatures(&[public_key], &chain, root, 3, &[signed.clone()]).is_ok());
        assert!(validate_import_signatures(&[], &chain, root, 3, &[]).is_ok());
        assert!(validate_import_signatures(&[public_key], &chain, root, 3, &[]).is_err());
        assert!(validate_import_signatures(&[Felt::ZERO], &chain, root, 3, &[signed.clone()]).is_err());
        assert!(validate_import_signatures(&[public_key], &ChainId::Mainnet, root, 3, &[signed.clone()]).is_err());
        assert!(validate_import_signatures(&[public_key], &chain, Felt::ONE, 3, &[signed.clone()]).is_err());
        assert!(validate_import_signatures(&[public_key], &chain, root, 4, &[signed.clone()]).is_err());
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

    #[test]
    fn committed_data_import_array_is_bounded_during_decoding() {
        assert!(serde_json::from_str::<CommittedDataValues>("[]").is_err());
        let encoded = serde_json::to_string(&vec![Felt::MAX, Felt::ZERO]).unwrap();
        let values: CommittedDataValues = serde_json::from_str(&encoded).unwrap();
        assert_eq!(values.0, vec![Felt::MAX, Felt::ZERO]);
        let count = blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES + 1;
        let encoded = format!("[{}]", std::iter::repeat_n("\"0x0\"", count).collect::<Vec<_>>().join(","));
        assert!(serde_json::from_str::<CommittedDataValues>(&encoded).is_err());
    }
}
