//! Domain-separated import signatures and signer-policy checks.
use super::{CommittedDataRpcError, ImportSignature};
use jsonrpsee::{core::RpcResult, types::ErrorObjectOwned};
use mp_convert::Felt;
use mp_gateway::committed_data::CommittedDataUpdate;
use std::collections::HashSet;

const MAX_SIGNATURES: usize = 32;
const IMPORT_SIGNATURE_DOMAIN: &[u8] = b"MADARA_COMMITTED_DATA_IMPORT_V1";
const UPDATE_SIGNATURE_DOMAIN: &[u8] = b"MADARA_COMMITTED_DATA_UPDATE_V1";

/// Canonical signing preimage: ASCII domain || chain-id felt (32 BE) || root (32 BE) || count (4 BE).
/// Starknet Keccak produces a 250-bit message supported by Stark ECDSA. Replay of the same dataset
/// is intentionally idempotent; there is no expiry or off-chain clock dependency.
pub fn import_message_hash(chain_id: Felt, root: Felt, count: u32) -> Felt {
    let mut bytes = IMPORT_SIGNATURE_DOMAIN.to_vec();
    bytes.extend_from_slice(&chain_id.to_bytes_be());
    bytes.extend_from_slice(&root.to_bytes_be());
    bytes.extend_from_slice(&count.to_be_bytes());
    starknet_core::utils::starknet_keccak(&bytes)
}

/// Empty policy disables authorization. Otherwise require 1..=32 distinct, allowed, valid signers.
pub(super) fn validate_import_signatures(
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
    let mut bytes = UPDATE_SIGNATURE_DOMAIN.to_vec();
    for value in [chain_id, update.application_id, update.dataset_id] {
        bytes.extend_from_slice(&value.to_bytes_be());
    }
    bytes.push(u8::from(update.base_root.is_some()));
    bytes.extend_from_slice(&update.base_root.unwrap_or(Felt::ZERO).to_bytes_be());
    bytes.extend_from_slice(&root.to_bytes_be());
    bytes.extend_from_slice(&count.to_be_bytes());
    starknet_core::utils::starknet_keccak(&bytes)
}

/// Apply the signer policy and reject duplicates, excessive work, or invalid ECDSA signatures.
pub(super) fn validate_signatures(allowed: &[Felt], message: Felt, signatures: &[ImportSignature]) -> RpcResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
