use jsonrpsee::core::RpcResult;
use jsonrpsee::types::{error::INVALID_PARAMS_CODE, ErrorCode, ErrorObjectOwned};
use mp_convert::Felt;
use mp_rpc::v0_10_0::{BlockId, BlockTag};

fn invalid_storage_key_error(key: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(ErrorCode::InvalidParams.code(), format!("Invalid storage key: {key}"), None::<()>)
}

pub(crate) fn validate_storage_proof_block_id(block_id: &BlockId) -> RpcResult<()> {
    if matches!(block_id, BlockId::Tag(BlockTag::PreConfirmed)) {
        return Err(ErrorObjectOwned::owned(INVALID_PARAMS_CODE, "Invalid params", None::<()>));
    }

    Ok(())
}

pub(crate) fn convert_storage_keys_for_v0_8_1(
    contracts_storage_keys: Option<Vec<mp_rpc::v0_10_0::ContractStorageKeysItem>>,
) -> RpcResult<Option<Vec<mp_rpc::v0_8_1::ContractStorageKeysItem>>> {
    contracts_storage_keys
        .map(|keys| {
            keys.into_iter()
                .map(|item| {
                    let storage_keys = item
                        .storage_keys
                        .into_iter()
                        .map(|key| Felt::from_hex(&key).map_err(|_| invalid_storage_key_error(&key)))
                        .collect::<Result<Vec<_>, _>>()?;

                    Ok(mp_rpc::v0_8_1::ContractStorageKeysItem {
                        contract_address: item.contract_address,
                        storage_keys,
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::{convert_storage_keys_for_v0_8_1, validate_storage_proof_block_id};
    use jsonrpsee::types::ErrorCode;
    use mp_rpc::v0_10_0::{BlockId, BlockTag};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[test]
    fn validate_storage_proof_block_id_rejects_pre_confirmed() {
        let error = validate_storage_proof_block_id(&BlockId::Tag(BlockTag::PreConfirmed))
            .expect_err("pre_confirmed should be rejected for getStorageProof");

        assert_eq!(error.code(), ErrorCode::InvalidParams.code());
        assert_eq!(error.message(), "Invalid params");
    }

    #[test]
    fn validate_storage_proof_block_id_accepts_latest() {
        assert!(validate_storage_proof_block_id(&BlockId::Tag(BlockTag::Latest)).is_ok());
    }

    #[test]
    fn convert_storage_keys_for_v0_8_1_accepts_none() {
        assert_eq!(convert_storage_keys_for_v0_8_1(None).unwrap(), None);
    }

    #[test]
    fn convert_storage_keys_for_v0_8_1_accepts_empty_contract_list() {
        let converted = convert_storage_keys_for_v0_8_1(Some(vec![])).unwrap().unwrap();
        assert!(converted.is_empty());
    }

    #[test]
    fn convert_storage_keys_for_v0_8_1_preserves_contract_and_key_order() {
        let converted = convert_storage_keys_for_v0_8_1(Some(vec![
            mp_rpc::v0_10_0::ContractStorageKeysItem {
                contract_address: Felt::ONE,
                storage_keys: vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()],
            },
            mp_rpc::v0_10_0::ContractStorageKeysItem { contract_address: Felt::TWO, storage_keys: vec![] },
            mp_rpc::v0_10_0::ContractStorageKeysItem {
                contract_address: Felt::THREE,
                storage_keys: vec!["0xa".to_string(), "0xb".to_string()],
            },
        ]))
        .unwrap()
        .unwrap();

        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].contract_address, Felt::ONE);
        assert_eq!(
            converted[0].storage_keys,
            vec![Felt::from_hex_unchecked("0x1"), Felt::from_hex_unchecked("0x2"), Felt::from_hex_unchecked("0x3")]
        );
        assert_eq!(converted[1].contract_address, Felt::TWO);
        assert!(converted[1].storage_keys.is_empty());
        assert_eq!(converted[2].contract_address, Felt::THREE);
        assert_eq!(converted[2].storage_keys, vec![Felt::from_hex_unchecked("0xa"), Felt::from_hex_unchecked("0xb")]);
    }

    #[rstest]
    #[case(vec!["not-a-felt".to_string()])]
    #[case(vec!["0x123".to_string(), "still-not-a-felt".to_string()])]
    fn convert_storage_keys_for_v0_8_1_rejects_invalid_storage_keys(#[case] storage_keys: Vec<String>) {
        let error = convert_storage_keys_for_v0_8_1(Some(vec![mp_rpc::v0_10_0::ContractStorageKeysItem {
            contract_address: Felt::ONE,
            storage_keys,
        }]))
        .unwrap_err();

        assert_eq!(error.code(), ErrorCode::InvalidParams.code());
        assert!(error.message().starts_with("Invalid storage key:"));
    }
}
