use mp_block::{BlockId, BlockTag};
use mp_rpc::BlockHashAndNumber;

use crate::errors::StarknetRpcResult;
use crate::{utils::OptionExt, Starknet};

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
    let block_info = starknet.get_block_info(&BlockId::Tag(BlockTag::Latest))?;
    let block_info = block_info.as_closed().ok_or_internal_server_error("Latest block is pending")?;

    Ok(BlockHashAndNumber { block_hash: block_info.block_hash, block_number: block_info.header.block_number })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{errors::StarknetRpcApiError, test_utils::rpc_test_setup};
    use mc_db::MadaraBackend;
    use mp_block::{
        header::PendingHeader, Header, MadaraBlockInfo, MadaraBlockInner, MadaraMaybePendingBlock,
        MadaraMaybePendingBlockInfo, MadaraPendingBlockInfo,
    };
    use mp_state_update::StateDiff;
    use rstest::rstest;
    use starknet_types_core::felt::Felt;

    #[rstest]
    fn test_block_hash_and_number(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;

        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header { parent_block_hash: Felt::ZERO, block_number: 0, ..Default::default() },
                        block_hash: Felt::ONE,
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                StateDiff::default(),
                vec![],
            )
            .unwrap();

        assert_eq!(block_hash_and_number(&rpc).unwrap(), BlockHashAndNumber { block_hash: Felt::ONE, block_number: 0 });

        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::NotPending(MadaraBlockInfo {
                        header: Header { parent_block_hash: Felt::ONE, block_number: 1, ..Default::default() },
                        block_hash: Felt::from_hex_unchecked("0x12345"),
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                StateDiff::default(),
                vec![],
            )
            .unwrap();

        assert_eq!(
            block_hash_and_number(&rpc).unwrap(),
            BlockHashAndNumber { block_hash: Felt::from_hex_unchecked("0x12345"), block_number: 1 }
        );

        // pending block should not be taken into account
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                        header: PendingHeader { parent_block_hash: Felt::ZERO, ..Default::default() },
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                StateDiff::default(),
                vec![],
            )
            .unwrap();

        assert_eq!(
            block_hash_and_number(&rpc).unwrap(),
            BlockHashAndNumber { block_hash: Felt::from_hex_unchecked("0x12345"), block_number: 1 }
        );
    }

    #[rstest]
    fn test_no_block_hash_and_number(rpc_test_setup: (Arc<MadaraBackend>, Starknet)) {
        let (backend, rpc) = rpc_test_setup;

        assert_eq!(block_hash_and_number(&rpc), Err(StarknetRpcApiError::BlockNotFound));

        // pending block should not be taken into account
        backend
            .store_block(
                MadaraMaybePendingBlock {
                    info: MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                        header: PendingHeader { parent_block_hash: Felt::ZERO, ..Default::default() },
                        tx_hashes: vec![],
                    }),
                    inner: MadaraBlockInner { transactions: vec![], receipts: vec![] },
                },
                StateDiff::default(),
                vec![],
            )
            .unwrap();

        assert_eq!(block_hash_and_number(&rpc), Err(StarknetRpcApiError::BlockNotFound));
    }
}
