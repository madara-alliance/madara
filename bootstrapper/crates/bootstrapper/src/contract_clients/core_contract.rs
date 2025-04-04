use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use ethers::abi::AbiEncode;
use ethers::addressbook::Address;
use ethers::prelude::{Bytes, I256, U256};
use starknet::core::types::Felt;
use starknet_proxy_client::interfaces::proxy::{CoreContractInitData, CoreContractState};
use zaun_utils::LocalWalletSignerMiddleware;

use crate::contract_clients::config::Clients;
use crate::utils::convert_felt_to_u256;

#[async_trait]
pub trait CoreContract {
    fn address(&self) -> Address;

    fn implementation_address(&self) -> Address;

    fn client(&self) -> Arc<LocalWalletSignerMiddleware>;

    async fn initialize_with(&self, init_data: CoreContractInitData);

    #[allow(clippy::too_many_arguments)]
    async fn add_implementation_core_contract(
        &self,
        block_number: Felt,
        state_root: Felt,
        block_hash: Felt,
        program_hash: Felt,
        config_hash: Felt,
        implementation_address: Address,
        verifier_address: Address,
        finalized: bool,
    );

    #[allow(clippy::too_many_arguments)]
    async fn upgrade_to_core_contract(
        &self,
        block_number: Felt,
        state_root: Felt,
        block_hash: Felt,
        program_hash: Felt,
        config_hash: Felt,
        implementation_address: Address,
        verifier_address: Address,
        finalized: bool,
    );

    async fn register_operator_core_contract(&self, operator_address: Address);

    async fn nominate_governor_core_contract(&self, l1_governor_address: Address);

    async fn nominate_governor_core_contract_proxy(&self, l1_governor_address: Address);

    async fn initialize(&self, program_hash: Felt, config_hash: Felt);

    async fn initialize_core_contract(
        &self,
        block_number: Felt,
        state_root: Felt,
        block_hash: Felt,
        program_hash: Felt,
        config_hash: Felt,
        verifer_address: Address,
    );
}

pub trait CoreContractDeploy<T> {
    fn deploy(clients: &Clients) -> impl Future<Output = T> + Send;
}

pub fn get_init_data_core_contract(
    block_number: Felt,
    state_root: Felt,
    block_hash: Felt,
    program_hash: Felt,
    config_hash: Felt,
    verifier_address: Address,
) -> CoreContractInitData {
    CoreContractInitData {
        program_hash: convert_felt_to_u256(program_hash), // zero program hash would be deemed invalid
        aggregate_program_hash: U256::zero(),
        verifier_address,
        config_hash: convert_felt_to_u256(config_hash),
        initial_state: CoreContractState {
            block_number: I256::from_raw(convert_felt_to_u256(block_number)),
            state_root: convert_felt_to_u256(state_root),
            block_hash: convert_felt_to_u256(block_hash),
        },
    }
}

pub fn get_calldata_bytes(calldata: CoreContractInitData) -> Bytes {
    let mut bytes_final = Address::zero().encode();
    let bytes: Vec<u8> = <CoreContractInitData as Into<Vec<u8>>>::into(calldata.clone());
    for x in bytes {
        bytes_final.push(x);
    }

    Bytes::from(bytes_final)
}
