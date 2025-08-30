use crate::errors::StarknetRpcResult;
use crate::Starknet;
use crate::StarknetRpcApiError;
use mp_rpc::v0_7_1::BlockHashAndNumber;

/// Get the Most Recent Accepted Block Hash and Number
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// * `block_hash_and_number` - A tuple containing the latest block hash and number of the current
///   network.
pub fn block_hash_and_number(starknet: &Starknet) -> StarknetRpcResult<BlockHashAndNumber> {
    let view = starknet.backend.block_view_on_last_confirmed().ok_or(StarknetRpcApiError::NoBlocks)?;
    let block_info = view.get_block_info()?;

    Ok(BlockHashAndNumber { block_hash: block_info.block_hash, block_number: block_info.header.block_number })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors::StarknetRpcApiError, test_utils::rpc_test_setup};
    use mc_db::{preconfirmed::PreconfirmedBlock, MadaraBackend};
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments};
    use rstest::rstest;
    use starknet_types_core::felt::Felt;
    use std::sync::Arc;

    #[rstest]
    fn test_block_hash_and_number(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 0, ..Default::default() },
                    ..Default::default()
                },
                &[],
                true,
            )
            .unwrap();

        assert_eq!(block_hash_and_number(&rpc).unwrap(), BlockHashAndNumber { block_hash: Felt::ONE, block_number: 0 });

        backend
            .write_access()
            .add_full_block_with_classes(
                &FullBlockWithoutCommitments {
                    header: PreconfirmedHeader { block_number: 1, ..Default::default() },
                    ..Default::default()
                },
                &[],
                true,
            )
            .unwrap();

        assert_eq!(
            block_hash_and_number(&rpc).unwrap(),
            BlockHashAndNumber { block_hash: Felt::from_hex_unchecked("0x12345"), block_number: 1 }
        );

        // pending block should not be taken into account

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: 2,
                ..Default::default()
            }))
            .unwrap();

        assert_eq!(
            block_hash_and_number(&rpc).unwrap(),
            BlockHashAndNumber { block_hash: Felt::from_hex_unchecked("0x12345"), block_number: 1 }
        );
    }

    #[rstest]
    fn test_no_block_hash_and_number(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;

        assert_eq!(block_hash_and_number(&rpc), Err(StarknetRpcApiError::NoBlocks));

        // pending block should not be taken into account
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader {
                block_number: 0,
                ..Default::default()
            }))
            .unwrap();

        assert_eq!(block_hash_and_number(&rpc), Err(StarknetRpcApiError::NoBlocks));
    }
}
