use std::sync::Arc;

use async_trait::async_trait;
use ethers::types::{Address, I256};
use starknet::core::types::Felt;
use starknet_core_contract_client::clients::StarknetDevCoreContractClient;
use starknet_core_contract_client::interfaces::{OperatorTrait, StarknetGovernanceTrait};
use starknet_core_contract_client::{
    deploy_starknet_core_contract_behind_proxy, CoreContractClientType, CoreContractType,
};
use starknet_proxy_client::deploy::ProxyVersion;
use starknet_proxy_client::interfaces::proxy::{CoreContractInitData, ProxyInitializeData, ProxySupport3_0_2Trait};
use zaun_utils::{LocalWalletSignerMiddleware, StarknetContractClient};

use crate::contract_clients::config::Clients;
use crate::contract_clients::core_contract::{
    get_calldata_bytes, get_init_data_core_contract, CoreContract, CoreContractDeploy,
};
use crate::utils::convert_felt_to_u256;

pub struct StarknetDevCoreContract {
    pub core_contract_client: StarknetDevCoreContractClient,
}

impl CoreContractDeploy<StarknetDevCoreContract> for StarknetDevCoreContract {
    async fn deploy(clients: &Clients) -> Self {
        let client = deploy_starknet_core_contract_behind_proxy(
            clients.eth_client().signer().clone(),
            ProxyVersion::SafeProxy3_0_2,
            CoreContractType::Dev,
        )
        .await
        .expect("Failed to deploy the starknet contact");

        match client {
            CoreContractClientType::Dev(c) => Self { core_contract_client: c },
            _ => {
                panic!("Unsupported client.")
            }
        }
    }
}

#[async_trait]
impl CoreContract for StarknetDevCoreContract {
    fn address(&self) -> Address {
        self.core_contract_client.address()
    }

    fn implementation_address(&self) -> Address {
        log::info!(
            "🎡 self.core_contract_client.implementation_address() : {:?}",
            self.core_contract_client.implementation_address()
        );
        self.core_contract_client.implementation_address()
    }

    fn client(&self) -> Arc<LocalWalletSignerMiddleware> {
        self.core_contract_client.client()
    }

    /// Initialize Starknet core contract with the specified data.
    /// IMP : only need to be called when using unsafe proxy
    async fn initialize_with(&self, init_data: CoreContractInitData) {
        let data = ProxyInitializeData::<0> { sub_contract_addresses: [], eic_address: Default::default(), init_data };

        log::info!("ℹ️  initialize_with : data : {:?}", data);

        self.core_contract_client.initialize_with(data).await.expect("Failed to initialize");

        self.core_contract_client
            .register_operator(self.core_contract_client.client().address())
            .await
            .expect("Failed to register operator");
    }

    /// Add implementation Starknet core contract with the specified data.
    #[allow(clippy::too_many_arguments)]
    async fn add_implementation_core_contract(
        &self,
        block_number: I256,
        state_root: Felt,
        block_hash: Felt,
        program_hash: Felt,
        config_hash: Felt,
        implementation_address: Address,
        verifier_address: Address,
        finalized: bool,
    ) {
        let init_data = get_init_data_core_contract(
            block_number,
            state_root,
            block_hash,
            program_hash,
            config_hash,
            verifier_address,
        );
        let final_bytes = get_calldata_bytes(init_data.clone());

        log::info!("ℹ️  add_implementation : data : {:?} : {:?}", init_data, final_bytes.clone());

        // https://sepolia.etherscan.io/tx/0x9ac02beb912e5c6226828110380d727a6fd7e4748cbded2198cdf62ea78dab62
        // let bytes_etherscan =
        // Bytes::from_str("
        // 0x0000000000000000000000000000000000000000000000000000000000000000001f0f5ba973c4d890fc2f1c18c33b87f487839fb44b9019e08d37d2ab943055000000000000000000000000f294781d719d2f4169ce54469c28908e6fa752c1044b9bae3e41deeeeeefe4dc6260a368e88458baab232bd5c89418063e4550c20000000000000000000000000000000000000000000000000000000000000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0000000000000000000000000000000000000000000000000000000000000000"
        // ).unwrap();

        self.core_contract_client
            .add_implementation(final_bytes, implementation_address, finalized)
            .await
            .expect("Failed to call add implementation");

        log::info!("ℹ️  add_implementation : done");
    }

    /// Add implementation Starknet core contract with the specified data.
    #[allow(clippy::too_many_arguments)]
    async fn upgrade_to_core_contract(
        &self,
        block_number: I256,
        state_root: Felt,
        block_hash: Felt,
        program_hash: Felt,
        config_hash: Felt,
        implementation_address: Address,
        verifier_address: Address,
        finalized: bool,
    ) {
        let init_data = get_init_data_core_contract(
            block_number,
            state_root,
            block_hash,
            program_hash,
            config_hash,
            verifier_address,
        );
        let final_bytes = get_calldata_bytes(init_data.clone());

        log::info!("ℹ️  upgrade_to : data : {:?} : {:?}", init_data, final_bytes.clone());

        // https://sepolia.etherscan.io/tx/0x9ac02beb912e5c6226828110380d727a6fd7e4748cbded2198cdf62ea78dab62
        // let bytes_etherscan =
        // Bytes::from_str("
        // 0x0000000000000000000000000000000000000000000000000000000000000000001f0f5ba973c4d890fc2f1c18c33b87f487839fb44b9019e08d37d2ab943055000000000000000000000000f294781d719d2f4169ce54469c28908e6fa752c1044b9bae3e41deeeeeefe4dc6260a368e88458baab232bd5c89418063e4550c20000000000000000000000000000000000000000000000000000000000000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff0000000000000000000000000000000000000000000000000000000000000000"
        // ).unwrap();

        self.core_contract_client
            .upgrade_to(final_bytes, implementation_address, finalized)
            .await
            .expect("Failed to call upgrade to");

        log::info!("ℹ️  upgrade_to : done");
    }

    /// For registering the operator for Starknet Core Contract
    async fn register_operator_core_contract(&self, operator_address: Address) {
        self.core_contract_client.register_operator(operator_address).await.expect("Failed to register operator");
        log::info!("ℹ️  register_operator : done");
    }

    /// For nominating the governor for Starknet Core Contract
    async fn nominate_governor_core_contract(&self, l1_governor_address: Address) {
        self.core_contract_client
            .starknet_nominate_new_governor(l1_governor_address)
            .await
            .expect("Failed to nominate governor");
        log::info!("ℹ️  register_operator : done");
    }

    /// For nominating the governor for Starknet Core Contract Proxy
    async fn nominate_governor_core_contract_proxy(&self, l1_governor_address: Address) {
        self.core_contract_client
            .proxy_nominate_new_governor(l1_governor_address)
            .await
            .expect("Failed to register operator");
        log::info!("ℹ️  proxy_nominate_new_governor : done");
    }

    /// Initialize Starknet core contract with the specified program and config hashes. The rest of
    /// parameters will be left default.
    /// IMP : only need to be called when using unsafe proxy
    async fn initialize(&self, program_hash: Felt, config_hash: Felt) {
        self.initialize_with(CoreContractInitData {
            program_hash: convert_felt_to_u256(program_hash),
            config_hash: convert_felt_to_u256(config_hash),
            ..Default::default()
        })
        .await;
    }

    /// Initialize Starknet core contract with the specified block number and state root hash.
    /// IMP : only need to be called when using unsafe proxy
    async fn initialize_core_contract(
        &self,
        block_number: I256,
        state_root: Felt,
        block_hash: Felt,
        program_hash: Felt,
        config_hash: Felt,
        verifer_address: Address,
    ) {
        let init_data = get_init_data_core_contract(
            block_number,
            state_root,
            block_hash,
            program_hash,
            config_hash,
            verifer_address,
        );

        self.initialize_with(init_data).await;
    }
}
