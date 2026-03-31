use crate::StarknetRpcApiError;
use mp_convert::Felt;

pub(crate) fn convert_storage_keys_for_v0_8_1(
    contracts_storage_keys: Option<Vec<mp_rpc::v0_10_0::ContractStorageKeysItem>>,
) -> Result<Option<Vec<mp_rpc::v0_8_1::ContractStorageKeysItem>>, StarknetRpcApiError> {
    contracts_storage_keys
        .map(|keys| {
            keys.into_iter()
                .map(|item| {
                    let storage_keys = item
                        .storage_keys
                        .into_iter()
                        .map(|key| {
                            Felt::from_hex(&key).map_err(|_| StarknetRpcApiError::ErrUnexpectedError {
                                error: format!("Invalid storage key: {key}").into(),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;

                    Ok(mp_rpc::v0_8_1::ContractStorageKeysItem {
                        contract_address: item.contract_address,
                        storage_keys,
                    })
                })
                .collect::<Result<Vec<_>, StarknetRpcApiError>>()
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::convert_storage_keys_for_v0_8_1;
    use crate::StarknetRpcApiError;
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[rstest]
    #[case(vec!["0x123".to_string()], Some(vec![Felt::from_hex_unchecked("0x123")]))]
    #[case(vec!["not-a-felt".to_string()], None)]
    fn convert_storage_keys_for_v0_8_1_handles_input(
        #[case] storage_keys: Vec<String>,
        #[case] expected: Option<Vec<Felt>>,
    ) {
        let result = convert_storage_keys_for_v0_8_1(Some(vec![mp_rpc::v0_10_0::ContractStorageKeysItem {
            contract_address: Felt::ONE,
            storage_keys,
        }]));

        match expected {
            Some(expected) => {
                let converted = result.unwrap().unwrap();
                assert_eq!(converted.len(), 1);
                assert_eq!(converted[0].contract_address, Felt::ONE);
                assert_eq!(converted[0].storage_keys, expected);
            }
            None => {
                let error = result.unwrap_err();
                let StarknetRpcApiError::ErrUnexpectedError { error } = error else {
                    panic!("expected invalid storage key error");
                };
                assert_eq!(error.as_ref(), "Invalid storage key: not-a-felt");
            }
        }
    }
}
