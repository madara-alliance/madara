use async_trait::async_trait;
use ethers::addressbook::Address;
use ethers::providers::Middleware;
use ethers::types::{Bytes, U256};
use starknet::accounts::{Account, ConnectedAccount};
use starknet::core::types::Felt;
use starknet_eth_bridge_client::clients::eth_bridge::StarknetEthBridgeContractClient;
use starknet_eth_bridge_client::interfaces::eth_bridge::StarknetEthBridgeTrait;
use starknet_eth_bridge_client::{
    deploy_starknet_eth_bridge_behind_safe_proxy, deploy_starknet_eth_bridge_behind_unsafe_proxy,
};
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::JsonRpcClient;
use starknet_proxy_client::interfaces::proxy::ProxySupport3_0_2Trait;
use std::sync::Arc;
use zaun_utils::{LocalWalletSignerMiddleware, StarknetContractClient};

use crate::contract_clients::utils::{field_element_to_u256, RpcAccount};
use crate::helpers::account_actions::{get_contract_address_from_deploy_tx, AccountActions};
use crate::utils::{invoke_contract, wait_for_transaction};

#[async_trait]
pub trait BridgeDeployable {
    async fn deploy(client: Arc<LocalWalletSignerMiddleware>, is_dev: bool) -> Self;
}

#[derive(Clone)]
pub struct StarknetLegacyEthBridge {
    eth_bridge: StarknetEthBridgeContractClient,
}

#[async_trait]
impl BridgeDeployable for StarknetLegacyEthBridge {
    async fn deploy(client: Arc<LocalWalletSignerMiddleware>, is_dev: bool) -> Self {
        let eth_bridge = match is_dev {
            true => deploy_starknet_eth_bridge_behind_unsafe_proxy(client.clone())
                .await
                .expect("Failed to deploy starknet contract"),
            false => deploy_starknet_eth_bridge_behind_safe_proxy(client.clone())
                .await
                .expect("Failed to deploy starknet contract"),
        };

        Self { eth_bridge }
    }
}

impl StarknetLegacyEthBridge {
    pub fn address(&self) -> Address {
        self.eth_bridge.address()
    }

    pub fn implementation_address(&self) -> Address {
        self.eth_bridge.implementation_address()
    }

    pub fn client(&self) -> Arc<LocalWalletSignerMiddleware> {
        self.eth_bridge.client()
    }

    pub async fn deploy_l2_contracts(
        rpc_provider_l2: &JsonRpcClient<HttpTransport>,
        legacy_eth_bridge_class_hash: Felt,
        legacy_eth_bridge_proxy_address: Felt,
        account: &RpcAccount<'_>,
    ) -> Felt {
        let legacy_eth_bridge_class_hash_correct = Felt::from_hex("0x78389bb177405c8f4f45e7397e15f2a86f94a1fe911a5efff9d481de596b364").unwrap();
        let deploy_tx = account
            .invoke_contract(
                account.address(),
                "deploy_contract",
                vec![legacy_eth_bridge_class_hash_correct, Felt::ZERO, Felt::ZERO, Felt::ZERO],
                None,
            )
            .send()
            .await
            .expect("Error deploying the contract proxy.");
        wait_for_transaction(
            rpc_provider_l2,
            deploy_tx.transaction_hash,
            "deploy_l2_contracts : deploy_contract : eth bridge",
        )
        .await
        .unwrap();
        let contract_address = get_contract_address_from_deploy_tx(account.provider(), &deploy_tx).await.unwrap();

        log::info!("🎡 contract address (eth bridge) : {:?}", contract_address);

        let add_implementation_txn = invoke_contract(
            legacy_eth_bridge_proxy_address,
            "add_implementation",
            vec![contract_address, Felt::ZERO, Felt::ONE, account.address(), Felt::ZERO],
            account,
        )
        .await;

        wait_for_transaction(
            rpc_provider_l2,
            add_implementation_txn.transaction_hash,
            "deploy_l2_contracts : add_implementation : eth bridge",
        )
        .await
        .unwrap();

        let upgrade_to_txn = invoke_contract(
            legacy_eth_bridge_proxy_address,
            "upgrade_to",
            vec![contract_address, Felt::ZERO, Felt::ONE, account.address(), Felt::ZERO],
            account,
        )
        .await;

        wait_for_transaction(
            rpc_provider_l2,
            upgrade_to_txn.transaction_hash,
            "deploy_l2_contracts : upgrade_to : eth bridge",
        )
        .await
        .unwrap();

        legacy_eth_bridge_proxy_address
    }

    /// Initialize Starknet Legacy Eth Bridge
    /// IMP : only need to be called when using unsafe proxy
    pub async fn initialize(&self, messaging_contract: Address) {
        let empty_bytes = [0u8; 32];

        let messaging_bytes = messaging_contract.as_bytes();

        let mut padded_messaging_bytes = Vec::with_capacity(32);
        padded_messaging_bytes.extend(vec![0u8; 32 - messaging_bytes.len()]);
        padded_messaging_bytes.extend_from_slice(messaging_bytes);

        let mut calldata = Vec::new();
        calldata.extend(empty_bytes);
        calldata.extend(empty_bytes);
        calldata.extend(padded_messaging_bytes);

        self.eth_bridge.initialize(Bytes::from(calldata)).await.expect("Failed to initialize eth bridge");
    }

    /// Add Implementation Starknet Legacy Eth Bridge
    pub async fn add_implementation_eth_bridge(&self, messaging_contract: Address) {
        let empty_bytes = [0u8; 32];

        let messaging_bytes = messaging_contract.as_bytes();

        let mut padded_messaging_bytes = Vec::with_capacity(32);
        padded_messaging_bytes.extend(vec![0u8; 32 - messaging_bytes.len()]);
        padded_messaging_bytes.extend_from_slice(messaging_bytes);

        let mut calldata = Vec::new();
        // `empty_bytes` act as an empty params for the calldata we are passing in bytes.
        // Here in this case of ETH Bridge it represents the EIC contract address, Token Address (ETH)
        // EIC = 0x0000000000000000000000000000000000000000
        // ETH Address to be passed in bridge = 0x0000000000000000000000000000000000000000
        calldata.extend(empty_bytes);
        calldata.extend(empty_bytes);
        calldata.extend(padded_messaging_bytes);

        log::info!("🎡 add_implementation_eth_bridge : bytes : {:?}", Bytes::from(calldata.clone()));

        self.eth_bridge
            .add_implementation(Bytes::from(calldata), self.implementation_address(), false)
            .await
            .expect("Failed to initialize eth bridge");
    }

    /// Upgrade To Starknet Legacy Eth Bridge
    pub async fn upgrade_to_eth_bridge(&self, messaging_contract: Address) {
        let empty_bytes = [0u8; 32];

        let messaging_bytes = messaging_contract.as_bytes();

        let mut padded_messaging_bytes = Vec::with_capacity(32);
        padded_messaging_bytes.extend(vec![0u8; 32 - messaging_bytes.len()]);
        padded_messaging_bytes.extend_from_slice(messaging_bytes);

        let mut calldata = Vec::new();
        // `empty_bytes` act as an empty params for the calldata we are passing in bytes.
        // Here in this case of ETH Bridge it represents the EIC contract address, Token Address (ETH)
        // EIC = 0x0000000000000000000000000000000000000000
        // ETH Address to be passed in bridge = 0x0000000000000000000000000000000000000000
        calldata.extend(empty_bytes);
        calldata.extend(empty_bytes);
        calldata.extend(padded_messaging_bytes);

        log::info!("🎡 upgrade_to_eth_bridge : bytes : {:?}", Bytes::from(calldata.clone()));

        self.eth_bridge
            .upgrade_to(Bytes::from(calldata), self.implementation_address(), false)
            .await
            .expect("Failed to initialize eth bridge");
    }

    /// Sets up the Eth bridge with the specified data
    pub async fn setup_l1_bridge(
        &self,
        max_total_balance: &str,
        max_deposit: &str,
        l2_bridge: Felt,
        l1_multisig_address: Address,
        is_dev: bool,
    ) {
        self.eth_bridge.set_max_total_balance(U256::from_dec_str(max_total_balance).unwrap()).await.unwrap();
        self.eth_bridge.set_max_deposit(U256::from_dec_str(max_deposit).unwrap()).await.unwrap();
        self.eth_bridge.set_l2_token_bridge(field_element_to_u256(l2_bridge)).await.unwrap();

        if !is_dev {
            // Nominating a new governor as l1 multi sig address
            self.eth_bridge.proxy_nominate_new_governor(l1_multisig_address).await.unwrap();
        }
    }

    pub async fn setup_l2_bridge(
        &self,
        rpc_provider: &JsonRpcClient<HttpTransport>,
        l2_bridge_address: Felt,
        erc20_address: Felt,
        l2_deployer_address: &str,
        account: &RpcAccount<'_>,
    ) {
        let tx = invoke_contract(l2_bridge_address, "set_l2_token", vec![erc20_address], account).await;

        log::info!("🎡 setup_l2_bridge : l2 token set //");
        wait_for_transaction(rpc_provider, tx.transaction_hash, "setup_l2_bridge : set_l2_token").await.unwrap();

        let tx = invoke_contract(
            l2_bridge_address,
            "set_l1_bridge",
            vec![Felt::from_bytes_be_slice(self.eth_bridge.address().as_bytes())],
            account,
        )
        .await;

        log::info!("🎡 setup_l2_bridge : l1 bridge set //");
        wait_for_transaction(rpc_provider, tx.transaction_hash, "setup_l2_bridge : set_l1_bridge").await.unwrap();
    }

    pub async fn set_max_total_balance(&self, amount: U256) {
        self.eth_bridge
            .set_max_total_balance(amount)
            .await
            .expect("Failed to set max total balance value in Eth bridge");
    }

    pub async fn set_max_deposit(&self, amount: U256) {
        self.eth_bridge.set_max_deposit(amount).await.expect("Failed to set max deposit value in eth bridge");
    }

    pub async fn set_l2_token_bridge(&self, l2_bridge: U256) {
        self.eth_bridge.set_l2_token_bridge(l2_bridge).await.expect("Failed to set l2 bridge in eth bridge");
    }

    pub async fn deposit(&self, amount: U256, l2_address: U256, fee: U256) {
        self.eth_bridge.deposit(amount, l2_address, fee).await.expect("Failed to deposit in eth bridge");
    }

    pub async fn withdraw(&self, amount: U256, l1_recipient: Address) {
        self.eth_bridge.withdraw(amount, l1_recipient).await.expect("Failed to withdraw from eth bridge");
    }

    pub async fn eth_balance(&self, l1_recipient: Address) -> U256 {
        let provider = self.eth_bridge.client().provider().clone();

        provider.get_balance(l1_recipient, None).await.unwrap()
    }
}
