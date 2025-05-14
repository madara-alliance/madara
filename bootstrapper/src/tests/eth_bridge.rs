use std::str::FromStr;
use std::time::Duration;

use ethers::prelude::U256;
use starknet::core::types::Felt;
use tokio::time::sleep;

use crate::contract_clients::config::Clients;
use crate::contract_clients::eth_bridge::StarknetLegacyEthBridge;
use crate::contract_clients::utils::read_erc20_balance;
use crate::tests::constants::L2_DEPLOYER_ADDRESS;
use crate::ConfigFile;
// TODO : Uncomment L2 --> L1 part after this part is added in the madara-orchestrator.

pub async fn eth_bridge_test_helper(
    clients: &Clients,
    arg_config: &ConfigFile,
    l2_eth_address: Felt,
    _l2_bridge_address: Felt,
    eth_bridge: StarknetLegacyEthBridge,
) -> Result<(), anyhow::Error> {
    let balance_before =
        read_erc20_balance(clients.provider_l2(), l2_eth_address, Felt::from_hex(L2_DEPLOYER_ADDRESS)?).await;

    eth_bridge.deposit(10.into(), U256::from_str(L2_DEPLOYER_ADDRESS)?, 1000.into()).await;

    log::info!("ETH deposited on l1 [💰]");
    sleep(Duration::from_secs(arg_config.cross_chain_wait_time)).await;
    sleep(Duration::from_secs((arg_config.l1_wait_time).parse()?)).await;
    log::info!("L1 message executed on L2 [🔁]");

    let balance_after =
        read_erc20_balance(clients.provider_l2(), l2_eth_address, Felt::from_hex(L2_DEPLOYER_ADDRESS)?).await;

    assert_eq!(balance_before[0] + Felt::from_dec_str("10")?, balance_after[0]);

    // Note: we are ignoring the withdrawal tests here, it would be part of e2e where
    // we have orch running as well

    // let l1_receipient = Felt::from_hex(&arg_config.l1_deployer_address).unwrap();
    //
    // sleep(Duration::from_secs((arg_config.l1_wait_time).parse()?)).await;
    //
    // let account =
    //     build_single_owner_account(clients.provider_l2(), &arg_config.rollup_priv_key,
    // L2_DEPLOYER_ADDRESS, false)         .await;
    //
    // invoke_contract(l2_bridge_address, "initiate_withdraw", vec![l1_receipient, Felt::from(5),
    // Felt::ZERO], &account)     .await;
    // log::info!("ETH withdrawal initiated on l2 [💰]");
    // log::info!("Waiting for message to be consumed on l2 [⏳]");
    // sleep(Duration::from_secs(arg_config.cross_chain_wait_time)).await;
    // sleep(Duration::from_secs((arg_config.l1_wait_time).parse()?)).await;
    //
    // let balance_before =
    // eth_bridge.eth_balance(Address::from_str(&arg_config.l1_deployer_address).unwrap()).await;
    // log::info!("Withdraw initiated on ETH Bridge [⏳]");
    // eth_bridge.withdraw(5.into(), Address::from_str(&arg_config.l1_deployer_address).unwrap()).await;
    // log::info!("Withdraw completed on ETH Bridge [✅]");
    // let balance_after =
    // eth_bridge.eth_balance(Address::from_str(&arg_config.l1_deployer_address).unwrap()).await;
    //
    // let decimals_eth = U256::from_dec_str("1000000000000000000").unwrap();
    //
    // assert_eq!(
    //     U256::checked_div(balance_before + U256::from_dec_str("5").unwrap(), decimals_eth).unwrap(),
    //     U256::checked_div(balance_after, decimals_eth).unwrap()
    // );

    Ok(())
}
