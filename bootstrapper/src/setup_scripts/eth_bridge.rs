use std::str::FromStr;

use ethers::abi::Address;
use serde::Serialize;
use starknet::accounts::{Account, ConnectedAccount};
use starknet::core::types::Felt;
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::JsonRpcClient;

use crate::contract_clients::config::Clients;
use crate::contract_clients::core_contract::CoreContract;
use crate::contract_clients::eth_bridge::{BridgeDeployable, StarknetLegacyEthBridge};
use crate::contract_clients::utils::{
    build_single_owner_account, declare_contract, deploy_proxy_contract, init_governance_proxy, DeclarationInput,
    RpcAccount,
};
use crate::helpers::account_actions::{get_contract_address_from_deploy_tx, AccountActions};
use crate::utils::constants::{ERC20_LEGACY_PATH, LEGACY_BRIDGE_PATH, PROXY_LEGACY_PATH, STARKGATE_PROXY_PATH};
use crate::utils::{
    convert_to_hex, hexstring_to_address, invoke_contract, save_to_json, wait_for_transaction, JsonValueType,
};
use crate::ConfigFile;

pub struct EthBridge<'a> {
    account: RpcAccount<'a>,
    account_address: Felt,
    arg_config: &'a ConfigFile,
    clients: &'a Clients,
    core_contract: &'a dyn CoreContract,
}

#[derive(Serialize, Clone)]
pub struct EthBridgeSetupOutput {
    pub l2_legacy_proxy_class_hash: Felt,
    pub l2_erc20_legacy_class_hash: Felt,
    pub l2_eth_proxy_address: Felt,
    pub l2_starkgate_proxy_class_hash: Felt,
    pub l2_legacy_eth_bridge_class_hash: Felt,
    pub l2_eth_bridge_proxy_address: Felt,
    pub l1_bridge_address: Address,
    #[serde(skip)]
    pub l1_bridge: StarknetLegacyEthBridge,
}

impl<'a> EthBridge<'a> {
    pub fn new(
        account: RpcAccount<'a>,
        account_address: Felt,
        arg_config: &'a ConfigFile,
        clients: &'a Clients,
        core_contract: &'a dyn CoreContract,
    ) -> Self {
        Self { account, account_address, arg_config, clients, core_contract }
    }

    pub async fn setup(&self) -> EthBridgeSetupOutput {
        // Declare a proxy
        let legacy_proxy_class_hash = declare_contract(DeclarationInput::LegacyDeclarationInputs(
            String::from(PROXY_LEGACY_PATH),
            self.arg_config.rollup_declare_v0_seq_url.clone(),
            self.clients.provider_l2(),
        ))
        .await;
        log::info!("🎡 Legacy proxy class hash declared.");
        save_to_json("legacy_proxy_class_hash", &JsonValueType::StringType(legacy_proxy_class_hash.to_string()))
            .unwrap();

        // Starkgate proxy declaration
        let starkgate_proxy_class_hash = declare_contract(DeclarationInput::LegacyDeclarationInputs(
            String::from(STARKGATE_PROXY_PATH),
            self.arg_config.rollup_declare_v0_seq_url.clone(),
            self.clients.provider_l2(),
        ))
        .await;
        log::info!("🎡 Starkgate proxy class hash declared.");
        save_to_json("starkgate_proxy_class_hash", &JsonValueType::StringType(starkgate_proxy_class_hash.to_string()))
            .unwrap();

        // Erc20 legacy class declaration
        let erc20_legacy_class_hash = declare_contract(DeclarationInput::LegacyDeclarationInputs(
            String::from(ERC20_LEGACY_PATH),
            self.arg_config.rollup_declare_v0_seq_url.clone(),
            self.clients.provider_l2(),
        ))
        .await;
        log::info!("🎡 ERC20 legacy class hash declared.");
        save_to_json("erc20_legacy_class_hash", &JsonValueType::StringType(erc20_legacy_class_hash.to_string()))
            .unwrap();

        let legacy_eth_bridge_class_hash = declare_contract(DeclarationInput::LegacyDeclarationInputs(
            String::from(LEGACY_BRIDGE_PATH),
            self.arg_config.rollup_declare_v0_seq_url.clone(),
            self.clients.provider_l2(),
        ))
        .await;
        log::info!("🎡 Legacy ETH Bridge class hash declared");
        save_to_json(
            "legacy_eth_bridge_class_hash",
            &JsonValueType::StringType(legacy_eth_bridge_class_hash.to_string()),
        )
        .unwrap();

        let eth_proxy_address = deploy_proxy_contract(
            &self.account,
            self.account_address,
            legacy_proxy_class_hash,
            // salt taken from : https://sepolia.starkscan.co/tx/0x06a5a493cf33919e58aa4c75777bffdef97c0e39cac968896d7bee8cc67905a1
            Felt::from_str("0x322c2610264639f6b2cee681ac53fa65c37e187ea24292d1b21d859c55e1a78").unwrap(),
            Felt::ONE,
        )
        .await;
        log::info!("✴️ ETH ERC20 proxy deployed [ETH : {:?}]", eth_proxy_address);
        save_to_json("l2_eth_address_proxy", &JsonValueType::StringType(eth_proxy_address.to_string())).unwrap();

        let eth_bridge_proxy_address = deploy_proxy_contract(
            &self.account,
            self.account_address,
            legacy_proxy_class_hash,
            Felt::from_str("0xabcdabcdabcd").unwrap(),
            Felt::ZERO,
        )
        .await;
        log::info!("✴️ ETH Bridge proxy deployed [ETH Bridge : {:?}]", eth_bridge_proxy_address);
        save_to_json("ETH_l2_bridge_address_proxy", &JsonValueType::StringType(eth_bridge_proxy_address.to_string()))
            .unwrap();

        init_governance_proxy(&self.account, eth_proxy_address, "eth_proxy_address : init_governance_proxy").await;

        init_governance_proxy(
            &self.account,
            eth_bridge_proxy_address,
            "eth_bridge_proxy_address : init_governance_proxy",
        )
        .await;

        let eth_bridge =
            StarknetLegacyEthBridge::deploy(self.core_contract.client().clone(), self.arg_config.dev).await;

        log::info!("✴️ ETH Bridge L1 deployment completed [Eth Bridge Address (L1) : {:?}]", eth_bridge.address());
        save_to_json("ETH_l1_bridge_address", &JsonValueType::EthAddress(eth_bridge.address())).unwrap();

        let account = build_single_owner_account(
            self.clients.provider_l2(),
            &self.arg_config.rollup_priv_key,
            &convert_to_hex(&self.account_address.to_string()),
            false,
        )
        .await;

        let l2_bridge_address = StarknetLegacyEthBridge::deploy_l2_contracts(
            self.clients.provider_l2(),
            legacy_eth_bridge_class_hash,
            eth_bridge_proxy_address,
            &account,
        )
        .await;

        log::info!("✴️ ETH Bridge L2 deployment completed [Eth Bridge Address (L2) : {:?}]", l2_bridge_address);
        save_to_json("ETH_l2_bridge_address", &JsonValueType::StringType(l2_bridge_address.to_string())).unwrap();

        // Deploy a token on l2 that will become the ETH
        let eth_address = deploy_eth_token_on_l2(
            self.clients.provider_l2(),
            eth_proxy_address,
            erc20_legacy_class_hash,
            &account,
            l2_bridge_address,
        )
        .await;

        log::info!("✴️ L2 ETH token deployment successful.");
        save_to_json("l2_eth_address", &JsonValueType::StringType(eth_address.to_string())).unwrap();
        if self.arg_config.dev {
            eth_bridge.initialize(self.core_contract.address()).await;
        } else {
            eth_bridge.add_implementation_eth_bridge(self.core_contract.address()).await;

            eth_bridge.upgrade_to_eth_bridge(self.core_contract.address()).await;
        }
        log::info!("✴️ ETH Bridge initialization on L1 completed");

        eth_bridge
            .setup_l2_bridge(
                self.clients.provider_l2(),
                l2_bridge_address,
                eth_address,
                &self.arg_config.rollup_priv_key,
                &account,
            )
            .await;
        log::info!("✴️ ETH Bridge initialization and setup on L2 completed");

        eth_bridge
            .setup_l1_bridge(
                "10000000000000000000000000000000000000000",
                "10000000000000000000000000000000000000000",
                l2_bridge_address,
                hexstring_to_address(&self.arg_config.l1_multisig_address),
                self.arg_config.dev,
            )
            .await;
        log::info!("✴️ ETH Bridge setup completed");

        EthBridgeSetupOutput {
            l2_legacy_proxy_class_hash: legacy_proxy_class_hash,
            l2_starkgate_proxy_class_hash: starkgate_proxy_class_hash,
            l2_erc20_legacy_class_hash: erc20_legacy_class_hash,
            l2_legacy_eth_bridge_class_hash: legacy_eth_bridge_class_hash,
            l2_eth_proxy_address: eth_proxy_address,
            l2_eth_bridge_proxy_address: eth_bridge_proxy_address,
            l1_bridge_address: eth_bridge.address(),
            l1_bridge: eth_bridge,
        }
    }
}

pub async fn deploy_eth_token_on_l2(
    rpc_provider_l2: &JsonRpcClient<HttpTransport>,
    eth_proxy_address: Felt,
    eth_erc20_class_hash: Felt,
    account: &RpcAccount<'_>,
    eth_legacy_bridge_address: Felt,
) -> Felt {
    // This is a workaournd for the SNOS bug where it incorrectly calculates cairo 0 class hash
    // Check function `get_real_class_hash_for_any_block` in `madara/crates/primitives/class/src/class_hash.rs` more details
    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support where one cant declare cairo 0 classes
    let eth_erc20_class_hash_correct =
        Felt::from_hex("0x2760f25d5a4fb2bdde5f561fd0b44a3dee78c28903577d37d669939d97036a0").unwrap();
    let deploy_tx = account
        .invoke_contract(
            account.address(),
            "deploy_contract",
            vec![eth_erc20_class_hash_correct, Felt::ZERO, Felt::ZERO, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error deploying the contract proxy.");
    wait_for_transaction(rpc_provider_l2, deploy_tx.transaction_hash, "deploy_eth_token_on_l2 : deploy").await.unwrap();
    let contract_address = get_contract_address_from_deploy_tx(account.provider(), &deploy_tx).await.unwrap();

    log::info!("Contract address (eth erc20) : {:?}", contract_address);

    let add_implementation_txn = invoke_contract(
        eth_proxy_address,
        "add_implementation",
        vec![
            contract_address,
            Felt::ZERO,
            Felt::from(4u64),
            Felt::from_bytes_be_slice("Ether".as_bytes()),
            Felt::from_bytes_be_slice("ETH".as_bytes()),
            Felt::from_str("18").unwrap(),
            eth_legacy_bridge_address,
            Felt::ZERO,
        ],
        account,
    )
    .await;

    wait_for_transaction(
        rpc_provider_l2,
        add_implementation_txn.transaction_hash,
        "deploy_eth_token_on_l2 : add_implementation",
    )
    .await
    .unwrap();

    let upgrade_to_txn = invoke_contract(
        eth_proxy_address,
        "upgrade_to",
        vec![
            contract_address,
            Felt::ZERO,
            Felt::from(4u64),
            Felt::from_bytes_be_slice("Ether".as_bytes()),
            Felt::from_bytes_be_slice("ETH".as_bytes()),
            Felt::from_str("18").unwrap(),
            eth_legacy_bridge_address,
            Felt::ZERO,
        ],
        account,
    )
    .await;

    wait_for_transaction(rpc_provider_l2, upgrade_to_txn.transaction_hash, "deploy_eth_token_on_l2 : upgrade_to")
        .await
        .unwrap();
    eth_proxy_address
}
