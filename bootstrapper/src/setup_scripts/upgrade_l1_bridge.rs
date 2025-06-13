use std::str::FromStr;
use std::sync::Arc;

use ethereum_instance::Error;
use ethers::prelude::{abigen, Bytes, SignerMiddleware};
use ethers::providers::{Http, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, U256};

use crate::utils::hexstring_to_address;
use crate::ConfigFile;

abigen!(
    EthereumL1BridgeProxy,
    r"[
        function addImplementation(address newImplementation, bytes data, bool finalize)
        function upgradeTo(address newImplementation, bytes data, bool finalize)
    ]",
);

abigen!(EthereumNewBridge, "../build-artifacts/starkgate_latest/eth_bridge_upgraded.json");
abigen!(EthereumNewBridgeEIC, "../build-artifacts/starkgate_latest/eic_eth_bridge.json");

/// Upgrades the L1 Ethereum bridge implementation with a new version, including deployment of new
/// contracts and configuration of administrative roles.
///
/// # Arguments
/// * `ethereum_bridge_address` - The address of the existing Ethereum bridge contract on L1
/// * `config_file` - Configuration file containing network and wallet settings
///
/// # Returns
/// * `Result<()>` - Result indicating success or failure of the upgrade process
///
/// # Steps
/// 1. Initializes provider and wallet connections using config settings
/// 2. Deploys new bridge implementation and EIC (External Implementation Contract)
/// 3. Sets up proxy connection to existing bridge
/// 4. Performs upgrade sequence:
///    - Adds new implementation to proxy
///    - Upgrades to new implementation
///    - Registers administrative roles (app role admin, governance admin, app governor)
///    - Sets maximum total balance for ETH
pub async fn upgrade_l1_bridge(ethereum_bridge_address: Address, config_file: &ConfigFile) -> color_eyre::Result<()> {
    let config_file = Arc::from(config_file);

    let provider = Provider::<Http>::try_from(config_file.eth_rpc.clone()).map_err(|_| Error::UrlParser)?;
    let wallet: LocalWallet = config_file.eth_priv_key.parse().expect("Failed to parse private key");
    let signer_client =
        Arc::new(SignerMiddleware::new(provider.clone(), wallet.with_chain_id(config_file.eth_chain_id)));

    let new_eth_bridge_client = EthereumNewBridge::deploy(signer_client.clone(), ())?.send().await?;
    log::debug!("New ETH bridge deployed : {:?}", new_eth_bridge_client.address());
    let eic_eth_bridge_client = EthereumNewBridgeEIC::deploy(signer_client.clone(), ())?.send().await?;
    log::debug!("New ETH bridge EIC deployed : {:?}", eic_eth_bridge_client.address());

    let eth_bridge_proxy_client = EthereumL1BridgeProxy::new(ethereum_bridge_address, signer_client.clone());

    // Building calldata :
    let client_clone = eic_eth_bridge_client.clone();
    let address = client_clone.address();
    let eic_eth_bridge_bytes = address.as_ref();
    // let eic_eth_bridge_bytes = eic_eth_bridge_client.address().as_ref();
    let mut padded_eic_eth_bridge_address = Vec::with_capacity(32);
    padded_eic_eth_bridge_address.extend(vec![0u8; 32 - eic_eth_bridge_bytes.len()]);
    padded_eic_eth_bridge_address.extend_from_slice(eic_eth_bridge_bytes);
    let empty_bytes = Bytes::from_str("0000000000000000000000000000000000000000000000000000000000000000")?;
    let call_data = [padded_eic_eth_bridge_address, empty_bytes.to_vec(), empty_bytes.to_vec()].concat();
    let call_data = Bytes::from(call_data);

    eth_bridge_proxy_client
        .add_implementation(new_eth_bridge_client.address(), call_data.clone(), false)
        .confirmations(2)
        .send()
        .await?;
    log::debug!("New ETH bridge add_implementation ✅");
    eth_bridge_proxy_client.upgrade_to(new_eth_bridge_client.address(), call_data, false).send().await?;
    log::debug!("New ETH bridge upgrade_to ✅");
    new_eth_bridge_client
        .register_app_role_admin(hexstring_to_address(&config_file.l1_deployer_address))
        .send()
        .await?;
    new_eth_bridge_client
        .register_governance_admin(hexstring_to_address(&config_file.l1_deployer_address))
        .send()
        .await?;
    new_eth_bridge_client.register_app_governor(hexstring_to_address(&config_file.l1_deployer_address)).send().await?;
    new_eth_bridge_client
        .set_max_total_balance(
            hexstring_to_address("0x0000000000000000000000000000000000455448"),
            U256::from_dec_str("10000000000000000000000000").unwrap(),
        )
        .send()
        .await?;
    log::debug!("New ETH bridge set_max_total_balance ✅");

    log::info!("Eth bridge L1 upgraded successfully ✅");
    Ok(())
}
