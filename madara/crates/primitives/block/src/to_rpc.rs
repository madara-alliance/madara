use mp_chain_config::StarknetVersion;

use crate::{header::PendingHeader, MadaraBlockInfo, MadaraPendingBlockInfo};

impl From<PendingHeader> for mp_rpc::v0_7_1::PendingBlockHeader {
    fn from(header: PendingHeader) -> Self {
        Self {
            l1_da_mode: header.l1_da_mode.into(),
            l1_data_gas_price: header.gas_prices.l1_data_gas_price(),
            l1_gas_price: header.gas_prices.l1_gas_price(),
            parent_hash: header.parent_block_hash,
            sequencer_address: header.sequencer_address,
            starknet_version: header.protocol_version.to_string(),
            timestamp: header.block_timestamp.0,
        }
    }
}

impl From<MadaraPendingBlockInfo> for mp_rpc::v0_7_1::PendingBlockHeader {
    fn from(MadaraPendingBlockInfo { header, .. }: MadaraPendingBlockInfo) -> Self {
        header.into()
    }
}

impl From<PendingHeader> for mp_rpc::v0_8_1::PendingBlockHeader {
    fn from(header: PendingHeader) -> Self {
        Self {
            l1_da_mode: header.l1_da_mode.into(),
            l1_data_gas_price: header.gas_prices.l1_data_gas_price(),
            l1_gas_price: header.gas_prices.l1_gas_price(),
            l2_gas_price: header.gas_prices.l2_gas_price(),
            parent_hash: header.parent_block_hash,
            sequencer_address: header.sequencer_address,
            starknet_version: if header.protocol_version < StarknetVersion::V0_9_1 {
                "".to_string()
            } else {
                header.protocol_version.to_string()
            },
            timestamp: header.block_timestamp.0,
        }
    }
}

impl From<MadaraPendingBlockInfo> for mp_rpc::v0_8_1::PendingBlockHeader {
    fn from(MadaraPendingBlockInfo { header, .. }: MadaraPendingBlockInfo) -> Self {
        header.into()
    }
}

impl From<MadaraBlockInfo> for mp_rpc::v0_7_1::BlockHeader {
    fn from(MadaraBlockInfo { header, block_hash, .. }: MadaraBlockInfo) -> Self {
        Self {
            block_hash,
            block_number: header.block_number,
            l1_da_mode: header.l1_da_mode.into(),
            l1_data_gas_price: header.gas_prices.l1_data_gas_price(),
            l1_gas_price: header.gas_prices.l1_gas_price(),
            new_root: header.global_state_root,
            parent_hash: header.parent_block_hash,
            sequencer_address: header.sequencer_address,
            starknet_version: if header.protocol_version < StarknetVersion::V0_9_1 {
                "".to_string()
            } else {
                header.protocol_version.to_string()
            },
            timestamp: header.block_timestamp.0,
        }
    }
}

impl From<MadaraBlockInfo> for mp_rpc::v0_8_1::BlockHeader {
    fn from(MadaraBlockInfo { header, block_hash, .. }: MadaraBlockInfo) -> Self {
        Self {
            block_hash,
            block_number: header.block_number,
            l1_da_mode: header.l1_da_mode.into(),
            l1_data_gas_price: header.gas_prices.l1_data_gas_price(),
            l1_gas_price: header.gas_prices.l1_gas_price(),
            l2_gas_price: header.gas_prices.l2_gas_price(),
            new_root: header.global_state_root,
            parent_hash: header.parent_block_hash,
            sequencer_address: header.sequencer_address,
            starknet_version: header.protocol_version.to_string(),
            timestamp: header.block_timestamp.0,
        }
    }
}
