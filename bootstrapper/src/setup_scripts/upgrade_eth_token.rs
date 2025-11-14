use starknet::accounts::{Account, ConnectedAccount};
use starknet::core::types::Felt;

use crate::contract_clients::config::Clients;
use crate::contract_clients::utils::{declare_contract, DeclarationInput, RpcAccount};
use crate::helpers::account_actions::{get_contract_address_from_deploy_tx, wait_at_least_block, AccountActions};
use crate::utils::constants::{
    EIC_ETH_TOKEN_CASM_PATH, EIC_ETH_TOKEN_SIERRA_PATH, NEW_ETH_TOKEN_CASM_PATH, NEW_ETH_TOKEN_SIERRA_PATH,
};
use crate::utils::wait_for_transaction;

/// Upgrades the Ethereum token contract implementation to Cairo 1 through a series of steps:
/// 1. Declares and deploys an ETH EIC (External Implementation Contract)
/// 2. Declares and deploys a new ETH token implementation
/// 3. Performs the upgrade process by:
///    - Adding the new implementation to the proxy
///    - Upgrading to the new implementation
///    - Registering governance and upgrade administrators
///    - Adding and replacing the new implementation class hash
/// 4. SNOS currently has a bug where if you upgrade a cairo 0 class to cairo 1, and interact with it in the same block,
///    the snos running for that block fails.
///    The workaround currently is that upgrade of cairo 0 happens in a separate block.
/// 5. For this sleeps are are added to ensure that the upgrade happens in a separate block.
///    This sleep delay time has to be greater than ideally the block time of the network while running the boostrapper.
///    The default block time is 10 seconds, so delays are added for 11 seconds.
/// 6. After boostrapper run is complete block time can be updated thus not affecting any generality.
///    This is a temporary fix and will be removed after boostrapper-v2 for starknet: v0.14.0
///    where cairo 0 classes cannot be declared
///
///
/// # Arguments
/// * `account` - The RPC account used to perform the transactions
/// * `rpc_provider_l2` - JSON-RPC client for L2 network communication
/// * `l2_eth_token_address` - The address of the existing ETH token contract on L2
pub async fn upgrade_eth_token_to_cairo_1(account: &RpcAccount<'_>, clients: &Clients, l2_eth_token_address: Felt) {
    let eth_eic_class_hash = declare_contract(
        clients,
        DeclarationInput::DeclarationInputs(
            String::from(EIC_ETH_TOKEN_SIERRA_PATH),
            String::from(EIC_ETH_TOKEN_CASM_PATH),
            account.clone(),
        ),
    )
    .await;
    log::debug!("ETH EIC declared ✅. Class hash : {:?}", eth_eic_class_hash);

    let new_eth_token_class_hash = declare_contract(
        clients,
        DeclarationInput::DeclarationInputs(
            String::from(NEW_ETH_TOKEN_SIERRA_PATH),
            String::from(NEW_ETH_TOKEN_CASM_PATH),
            account.clone(),
        ),
    )
    .await;
    log::debug!("New ETH token declared ✅. Class hash : {:?}", new_eth_token_class_hash);

    let eth_eic_deploy_tx = account
        .invoke_contract(
            account.address(),
            "deploy_contract",
            vec![eth_eic_class_hash, Felt::ZERO, Felt::ZERO, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error deploying the contract : eth_eic_deploy_tx");
    wait_for_transaction(clients.provider_l2(), eth_eic_deploy_tx.transaction_hash, "deploy_eth_token_on_l2 : deploy")
        .await
        .unwrap();
    let eth_eic_contract_address =
        get_contract_address_from_deploy_tx(account.provider(), &eth_eic_deploy_tx).await.unwrap();
    log::debug!("✅ eth eic contract address : {:?}", eth_eic_contract_address);

    let new_token_eth_deploy_tx = account
        .invoke_contract(
            account.address(),
            "deploy_contract",
            vec![
                new_eth_token_class_hash,
                Felt::ZERO,
                Felt::ZERO,
                Felt::from(9u64),
                Felt::from_hex("eee").unwrap(),
                Felt::from_hex("eeee").unwrap(),
                Felt::from(6u64),
                Felt::from(0),
                Felt::from(0),
                Felt::from_hex("137e2eb39d5b20f7257425dbea0a97ab6a53941e7ccdc9168ba3b0f8b39d1ce").unwrap(),
                Felt::from_hex("137e2eb39d5b20f7257425dbea0a97ab6a53941e7ccdc9168ba3b0f8b39d1ce").unwrap(),
                Felt::from_hex("137e2eb39d5b20f7257425dbea0a97ab6a53941e7ccdc9168ba3b0f8b39d1ce").unwrap(),
                Felt::from(0),
            ],
            None,
        )
        .send()
        .await
        .expect("Error deploying the contract : new_token_eth_deploy_tx");
    wait_for_transaction(
        clients.provider_l2(),
        new_token_eth_deploy_tx.transaction_hash,
        "deploy_eth_token_on_l2 : deploy",
    )
    .await
    .unwrap();
    let new_eth_token_contract_address =
        get_contract_address_from_deploy_tx(account.provider(), &new_token_eth_deploy_tx).await.unwrap();
    log::debug!("✅ new eth contract address : {:?}", new_eth_token_contract_address);

    let eth_token_add_implementation_new_txn = account
        .invoke_contract(
            l2_eth_token_address,
            "add_implementation",
            vec![new_eth_token_contract_address, eth_eic_contract_address, Felt::ZERO, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(
        clients.provider_l2(),
        eth_token_add_implementation_new_txn.transaction_hash,
        "Interact ETH token",
    )
    .await
    .unwrap();

    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_token_to_cairo_1` for more details
    wait_at_least_block(clients.provider_l2(), Some(1)).await;

    log::debug!(
        "upgrade_eth_token_to_cairo_1 : add implementation : eth proxy ✅, Txn hash : {:?}",
        eth_token_add_implementation_new_txn.transaction_hash
    );

    let eth_token_upgrade_to_new_txn = account
        .invoke_contract(
            l2_eth_token_address,
            "upgrade_to",
            vec![new_eth_token_contract_address, eth_eic_contract_address, Felt::ZERO, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(clients.provider_l2(), eth_token_upgrade_to_new_txn.transaction_hash, "Interact ETH token")
        .await
        .unwrap();

    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_token_to_cairo_1` for more details
    wait_at_least_block(clients.provider_l2(), Some(1)).await;

    log::debug!(
        "upgrade_eth_token_to_cairo_1 : upgrade to : eth proxy ✅, Txn hash : {:?}",
        eth_token_upgrade_to_new_txn.transaction_hash
    );

    let eth_token_register_governance_admin_txn = account
        .invoke_contract(l2_eth_token_address, "register_governance_admin", vec![account.address()], None)
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(
        clients.provider_l2(),
        eth_token_register_governance_admin_txn.transaction_hash,
        "Interact ETH token",
    )
    .await
    .unwrap();
    log::debug!(
        "upgrade_eth_token_to_cairo_1 : register_governance_admin : eth proxy ✅, Txn hash : {:?}",
        eth_token_register_governance_admin_txn.transaction_hash
    );

    let eth_token_register_upgrade_governor_txn = account
        .invoke_contract(l2_eth_token_address, "register_upgrade_governor", vec![account.address()], None)
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(
        clients.provider_l2(),
        eth_token_register_upgrade_governor_txn.transaction_hash,
        "Interact ETH token",
    )
    .await
    .unwrap();
    log::debug!(
        "upgrade_eth_token_to_cairo_1 : register_upgrade_governor : eth proxy ✅, Txn hash : {:?}",
        eth_token_register_upgrade_governor_txn.transaction_hash
    );

    let new_eth_token_add_implementation_txn = account
        .invoke_contract(
            l2_eth_token_address,
            "add_new_implementation",
            vec![new_eth_token_class_hash, Felt::ONE, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(
        clients.provider_l2(),
        new_eth_token_add_implementation_txn.transaction_hash,
        "Interact ETH token",
    )
    .await
    .unwrap();

    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_token_to_cairo_1` for more details
    wait_at_least_block(clients.provider_l2(), Some(1)).await;

    log::debug!(
        "upgrade_eth_token_to_cairo_1 : add_new_implementation : eth proxy ✅, Txn hash : {:?}",
        new_eth_token_add_implementation_txn.transaction_hash
    );

    let new_eth_token_replace_to_txn = account
        .invoke_contract(
            l2_eth_token_address,
            "replace_to",
            vec![new_eth_token_class_hash, Felt::ONE, Felt::ZERO],
            None,
        )
        .send()
        .await
        .expect("Error calling eth token proxy");
    wait_for_transaction(clients.provider_l2(), new_eth_token_replace_to_txn.transaction_hash, "Interact ETH token")
        .await
        .unwrap();

    // This is a temperary workaround which can be removed after starknet: v0.14.0 boostrapper support
    // where cairo 0 classes cannot be declared
    // Refer the description in `upgrade_eth_token_to_cairo_1` for more details
    wait_at_least_block(clients.provider_l2(), Some(1)).await;

    log::debug!(
        "upgrade_eth_token_to_cairo_1 : replace_to : eth proxy ✅, Txn hash : {:?}",
        new_eth_token_replace_to_txn.transaction_hash
    );

    log::info!("Eth token upgraded successfully ✅");
}
