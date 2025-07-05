use starknet::accounts::{Account, ConnectedAccount};
use starknet_providers::jsonrpc::HttpTransport;
use starknet_providers::JsonRpcClient;
use starknet_types_core::felt::Felt;
use std::time::Duration;
use tokio::time::sleep;

use crate::contract_clients::utils::{declare_contract, DeclarationInput, RpcAccount};
use crate::helpers::account_actions::{get_contract_address_from_deploy_tx, AccountActions};
use crate::utils::constants::{
    EIC_ETH_BRIDGE_CASM_PATH, EIC_ETH_BRIDGE_SIERRA_PATH, NEW_ETH_BRIDGE_CASM_PATH, NEW_ETH_BRIDGE_SIERRA_PATH,
};
use crate::utils::wait_for_transaction;

/// Upgrades the L2 Ethereum bridge implementation to Cairo 1 through a sequence of contract
/// declarations, deployments, and configuration steps.
///
/// # Arguments
/// * `account` - The RPC account used to perform the transactions
/// * `rpc_provider_l2` - JSON-RPC client for L2 network communication
/// * `l2_eth_bridge_address` - The address of the existing ETH bridge contract on L2
/// * `l2_eth_token_address` - The address of the ETH token contract on L2
///
/// # Steps
/// 1. Declares and deploys bridge EIC (External Implementation Contract)
/// 2. Declares and deploys new bridge implementation
/// 3. Executes upgrade sequence:
///    - Adds new implementation to proxy with ETH token configuration
///    - Upgrades to new implementation
///    - Registers governance and upgrade administrators
///    - Adds and replaces implementation class hash
/// 4. SNOS currently has a bug where if you upgrade a cairo 0 class to cairo 1, and interact with it in the same block,
///    the snos running for that block fails.
///    The workaround currently is that upgrade of cairo 0 happens in a separate block.
/// 5. For this sleeps are are added to ensure that the upgrade happens in a separate block.
///    This sleep delay time has to be greater than ideally the block time of the network while running the boostrapper.
///    The default block time is 10 seconds, so delays are added for 11 seconds.
/// 6. After boostrapper run is complete block can be updated not affect any generality.
///    This is a temporary fix and will be removed after boostrapper-v2 for starknet: v0.14.0
///    where cairo 0 classes cannot be declared
pub async fn upgrade_eth_bridge_to_cairo_1(
    account: &RpcAccount<'_>,
    rpc_provider_l2: &JsonRpcClient<HttpTransport>,
    l2_eth_bridge_address: Felt,
    l2_eth_token_address: Felt,
) {
    let eth_bridge_eic_class_hash = declare_contract(DeclarationInput::DeclarationInputs(
        String::from(EIC_ETH_BRIDGE_SIERRA_PATH),
        String::from(EIC_ETH_BRIDGE_CASM_PATH),
        account.clone(),
    ))
    .await;
    log::debug!("ETH Bridge EIC declared ✅, Class hash : {:?}", eth_bridge_eic_class_hash);

    let new_eth_bridge_class_hash = declare_contract(DeclarationInput::DeclarationInputs(
        String::from(NEW_ETH_BRIDGE_SIERRA_PATH),
        String::from(NEW_ETH_BRIDGE_CASM_PATH),
        account.clone(),
    ))
    .await;
    log::debug!("New ETH Bridge declared ✅, Class hash : {:?}", new_eth_bridge_class_hash);

    let bridge_eic_deploy_tx = account
        .invoke_contract(
            account.address(),
            "deploy_contract",
            vec![eth_bridge_eic_class_hash, Felt::ONE, Felt::ZERO, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error deploying the contract : eth_eic_deploy_tx");
    wait_for_transaction(rpc_provider_l2, bridge_eic_deploy_tx.transaction_hash, " : deploy").await.unwrap();
    let eth_bridge_eic_contract_address =
        get_contract_address_from_deploy_tx(account.provider(), &bridge_eic_deploy_tx).await.unwrap();
    log::debug!("✅ eth bridge eic contract address : {:?}", eth_bridge_eic_contract_address);

    let new_bridge_eth_deploy_tx = account
        .invoke_contract(
            account.address(),
            "deploy_contract",
            vec![new_eth_bridge_class_hash, Felt::ONE, Felt::ZERO, Felt::from(1u64), Felt::from(0)],
            None,
        )
        .send()
        .await
        .expect("Error deploying the contract : new_token_eth_deploy_tx");
    wait_for_transaction(rpc_provider_l2, new_bridge_eth_deploy_tx.transaction_hash, " : deploy").await.unwrap();
    let new_eth_bridge_contract_address =
        get_contract_address_from_deploy_tx(account.provider(), &new_bridge_eth_deploy_tx).await.unwrap();
    log::debug!("✅ new eth bridge contract address : {:?}", new_eth_bridge_contract_address);

    let eth_bridge_add_implementation_txn = account
        .invoke_contract(
            l2_eth_bridge_address,
            "add_implementation",
            vec![
                new_eth_bridge_contract_address,
                eth_bridge_eic_contract_address,
                Felt::TWO,
                Felt::from_hex("455448").unwrap(),
                l2_eth_token_address,
                Felt::ZERO,
            ],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(rpc_provider_l2, eth_bridge_add_implementation_txn.transaction_hash, "Interact ETH bridge")
        .await
        .unwrap();
    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_bridge_to_cairo_1` for more details
    sleep(Duration::from_secs(11)).await;
    log::debug!(
        "upgrade_eth_bridge_to_cairo_1 : add_implementation : eth bridge ✅, Txn hash : {:?}",
        eth_bridge_add_implementation_txn.transaction_hash
    );

    let eth_bridge_upgrade_to_txn = account
        .invoke_contract(
            l2_eth_bridge_address,
            "upgrade_to",
            vec![
                new_eth_bridge_contract_address,
                eth_bridge_eic_contract_address,
                Felt::TWO,
                Felt::from_hex("455448").unwrap(),
                l2_eth_token_address,
                Felt::ZERO,
            ],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(rpc_provider_l2, eth_bridge_upgrade_to_txn.transaction_hash, "Interact ETH bridge")
        .await
        .unwrap();
    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_bridge_to_cairo_1` for more details
    sleep(Duration::from_secs(11)).await;
    log::debug!(
        "upgrade_eth_bridge_to_cairo_1 : upgrade_to : eth bridge ✅, Txn hash : {:?}",
        eth_bridge_upgrade_to_txn.transaction_hash
    );

    let eth_bridge_register_governance_admin_txn = account
        .invoke_contract(l2_eth_bridge_address, "register_governance_admin", vec![account.address()], None)
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(
        rpc_provider_l2,
        eth_bridge_register_governance_admin_txn.transaction_hash,
        "Interact ETH bridge",
    )
    .await
    .unwrap();
    log::debug!(
        "upgrade_eth_bridge_to_cairo_1 : register_governance_admin : eth bridge ✅, Txn hash : {:?}",
        eth_bridge_register_governance_admin_txn.transaction_hash
    );

    let eth_bridge_register_upgrade_governor_txn = account
        .invoke_contract(l2_eth_bridge_address, "register_upgrade_governor", vec![account.address()], None)
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(
        rpc_provider_l2,
        eth_bridge_register_upgrade_governor_txn.transaction_hash,
        "Interact ETH bridge",
    )
    .await
    .unwrap();
    log::debug!(
        "upgrade_eth_bridge_to_cairo_1 : register_upgrade_governor : eth bridge ✅, Txn hash : {:?}",
        eth_bridge_register_upgrade_governor_txn.transaction_hash
    );

    let eth_bridge_add_new_implementation_txn = account
        .invoke_contract(
            l2_eth_bridge_address,
            "add_new_implementation",
            vec![new_eth_bridge_class_hash, Felt::ONE, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(rpc_provider_l2, eth_bridge_add_new_implementation_txn.transaction_hash, "Interact ETH token")
        .await
        .unwrap();
    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_bridge_to_cairo_1` for more details
    sleep(Duration::from_secs(11)).await;
    log::debug!(
        "upgrade_eth_bridge_to_cairo_1 : add_new_implementation : eth bridge ✅, Txn hash : {:?}",
        eth_bridge_add_new_implementation_txn.transaction_hash
    );

    let eth_bridge_replace_to_txn = account
        .invoke_contract(
            l2_eth_bridge_address,
            "replace_to",
            vec![new_eth_bridge_class_hash, Felt::ONE, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(rpc_provider_l2, eth_bridge_replace_to_txn.transaction_hash, "Interact ETH token")
        .await
        .unwrap();
    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_bridge_to_cairo_1` for more details
    sleep(Duration::from_secs(11)).await;
    log::debug!(
        "upgrade_eth_bridge_to_cairo_1 : replace_to : eth bridge ✅, Txn hash : {:?}",
        eth_bridge_replace_to_txn.transaction_hash
    );

    log::info!("Eth bridge L2 upgraded successfully ✅");
}
