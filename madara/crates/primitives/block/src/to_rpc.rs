use crate::{header::PreconfirmedHeader, Header, MadaraBlockInfo};
use mp_convert::Felt;

impl PreconfirmedHeader {
    pub fn to_rpc_v0_7(&self, parent_hash: Felt) -> mp_rpc::v0_7_1::PendingBlockHeader {
        mp_rpc::v0_7_1::PendingBlockHeader {
            l1_da_mode: self.l1_da_mode.into(),
            l1_data_gas_price: self.gas_prices.l1_data_gas_price(),
            l1_gas_price: self.gas_prices.l1_gas_price(),
            parent_hash,
            sequencer_address: self.sequencer_address,
            starknet_version: self.protocol_version.to_string(),
            timestamp: self.block_timestamp.0,
        }
    }
    pub fn to_rpc_v0_8(&self, parent_hash: Felt) -> mp_rpc::v0_8_1::PendingBlockHeader {
        mp_rpc::v0_8_1::PendingBlockHeader {
            l1_da_mode: self.l1_da_mode.into(),
            l1_data_gas_price: self.gas_prices.l1_data_gas_price(),
            l1_gas_price: self.gas_prices.l1_gas_price(),
            l2_gas_price: self.gas_prices.l2_gas_price(),
            parent_hash,
            sequencer_address: self.sequencer_address,
            starknet_version: self.protocol_version.to_string(),
            timestamp: self.block_timestamp.0,
        }
    }
    pub fn to_rpc_v0_9(&self) -> mp_rpc::v0_9_0::PreConfirmedBlockHeader {
        mp_rpc::v0_9_0::PreConfirmedBlockHeader {
            l1_da_mode: self.l1_da_mode.into(),
            l1_data_gas_price: self.gas_prices.l1_data_gas_price(),
            l1_gas_price: self.gas_prices.l1_gas_price(),
            l2_gas_price: self.gas_prices.l2_gas_price(),
            block_number: self.block_number,
            sequencer_address: self.sequencer_address,
            starknet_version: self.protocol_version.to_string(),
            timestamp: self.block_timestamp.0,
        }
    }
}

impl MadaraBlockInfo {
    pub fn to_rpc_v0_7(&self) -> mp_rpc::v0_7_1::BlockHeader {
        self.header.to_rpc_v0_7(self.block_hash)
    }
    pub fn to_rpc_v0_8(&self) -> mp_rpc::v0_8_1::BlockHeader {
        self.header.to_rpc_v0_8(self.block_hash)
    }
}

impl Header {
    pub fn to_rpc_v0_7(&self, block_hash: Felt) -> mp_rpc::v0_7_1::BlockHeader {
        mp_rpc::v0_7_1::BlockHeader {
            l1_da_mode: self.l1_da_mode.into(),
            l1_data_gas_price: self.gas_prices.l1_data_gas_price(),
            l1_gas_price: self.gas_prices.l1_gas_price(),
            parent_hash: self.parent_block_hash,
            sequencer_address: self.sequencer_address,
            starknet_version: self.protocol_version.to_string(),
            timestamp: self.block_timestamp.0,
            block_hash,
            block_number: self.block_number,
            new_root: self.global_state_root,
        }
    }
    pub fn to_rpc_v0_8(&self, block_hash: Felt) -> mp_rpc::v0_8_1::BlockHeader {
        mp_rpc::v0_8_1::BlockHeader {
            l1_da_mode: self.l1_da_mode.into(),
            l1_data_gas_price: self.gas_prices.l1_data_gas_price(),
            l1_gas_price: self.gas_prices.l1_gas_price(),
            l2_gas_price: self.gas_prices.l2_gas_price(),
            parent_hash: self.parent_block_hash,
            sequencer_address: self.sequencer_address,
            starknet_version: self.protocol_version.to_string(),
            timestamp: self.block_timestamp.0,
            block_hash,
            block_number: self.block_number,
            new_root: self.global_state_root,
        }
    }
}
