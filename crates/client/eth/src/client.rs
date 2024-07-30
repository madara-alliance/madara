use std::sync::Arc;

use alloy::{
    primitives::{Address},
    providers::{Provider, ProviderBuilder, ReqwestProvider, RootProvider},
    rpc::types::Filter,
    sol,
    transports::http::{Client, Http},
};
use alloy::sol_types::SolEvent;
use anyhow::{bail, Context};
use bitvec::macros::internal::funty::Fundamental;
use starknet_api::hash::StarkFelt;
use url::Url;

use crate::{
    config::L1StateUpdate,
    utils::{u256_to_starkfelt},
};
use crate::client::StarknetCoreContract::StarknetCoreContractInstance;


sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetCoreContract,
    "src/abis/starknet_core_new.json"
);

pub struct EthereumClient {
    pub provider: Arc<ReqwestProvider>,
    pub l1_core_contract: StarknetCoreContractInstance<Http<Client>, RootProvider<Http<Client>>>,
}

impl EthereumClient {
    /// Create a new EthereumClient instance with the given RPC URL
    pub async fn new(url: Url, l1_core_address: Address) -> anyhow::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url);
        let core_contract = StarknetCoreContract::new(l1_core_address, provider.clone());

        Ok(Self { provider: Arc::new(provider), l1_core_contract: core_contract })
    }

    /// Retrieves the latest Ethereum block number
    pub async fn get_latest_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.provider.get_block_number().await?.as_u64();
        Ok(block_number)
    }

    /// Get the block number of the last occurrence of a given event.
    pub async fn get_last_event_block_number<T: SolEvent>(&self) -> anyhow::Result<u64> {
        let latest_block: u64 = self.get_latest_block_number().await?;

        let filter = Filter::new()
            .from_block(latest_block - 6000)
            .to_block(latest_block)
            .address(*self.l1_core_contract.address());

        let logs = self.provider
            .get_logs(&filter)
            .await?;

        let filtered_logs = logs
            .clone()
            .into_iter()
            .filter_map(|log| {
                log.log_decode::<T>().ok()
            }).collect::<Vec<_>>();

        if let Some(last_log) = filtered_logs.last() {
            let last_block: u64 = last_log.block_number.context("no block number in log")?;
            Ok(last_block)
        } else {
            bail!("no event found")
        }
    }

    /// Get the last Starknet block number verified on L1
    pub async fn get_last_verified_block_number(&self) -> anyhow::Result<u64> {
        let block_number = self.l1_core_contract.stateBlockNumber().call().await?;
        let last_block_number: u64 = (block_number._0).as_u64();
        Ok(last_block_number)
    }

    /// Get the last Starknet state root verified on L1
    pub async fn get_last_state_root(&self) -> anyhow::Result<StarkFelt> {
        let state_root = self.l1_core_contract.stateRoot().call().await?;
        println!("state root is: {:?}", state_root._0.clone());
        u256_to_starkfelt(state_root._0)
    }

    /// Get the last Starknet block hash verified on L1
    pub async fn get_last_verified_block_hash(&self) -> anyhow::Result<StarkFelt> {
        let block_hash = self.l1_core_contract.stateBlockHash().call().await?;
        u256_to_starkfelt(block_hash._0)
    }

    /// Get the last Starknet state update verified on the L1
    pub async fn get_initial_state(client: &EthereumClient) -> anyhow::Result<L1StateUpdate> {
        let block_number = client.get_last_verified_block_number().await?;
        let block_hash = client.get_last_verified_block_hash().await?;
        let global_root = client.get_last_state_root().await?;

        Ok(L1StateUpdate { global_root, block_number, block_hash })
    }
}


#[cfg(test)]
mod eth_client_test {
    use alloy::node_bindings::Anvil;
    use super::*;
    use tokio;
    use alloy::primitives::{address, U256};

    #[tokio::test]
    async fn test_get_latest_block_number() {
        let anvil = Anvil::new().fork("https://eth.merkle.io").fork_block_number(20395662).try_spawn().expect("issue while forking");
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider =
            ProviderBuilder::new().on_http(rpc_url.clone());
        let contract = StarknetCoreContract::new(address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4"), provider.clone());

        let eth_client = EthereumClient {
            provider:Arc::new(provider),
            l1_core_contract: contract.clone()
        };
        let block_number = eth_client.provider.get_block_number().await.expect("issue while fetching the block number").as_u64();
        assert_eq!(block_number, 20395662, "provider unable to get the correct block number");
    }

    #[tokio::test]
    async fn test_get_last_event_block_number() {
        let anvil = Anvil::new().fork("https://eth.merkle.io").fork_block_number(20395662).try_spawn().expect("issue while forking");
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider =
            ProviderBuilder::new().on_http(rpc_url.clone());
        let contract = StarknetCoreContract::new(address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4"), provider.clone());

        let eth_client = EthereumClient {
            provider:Arc::new(provider),
            l1_core_contract: contract.clone()
        };
        let block_number = eth_client.get_last_event_block_number::<StarknetCoreContract::LogStateUpdate>().await.expect("issue while getting the last block number with given event");
        assert_eq!(block_number, 20395662, "block number with given event not matching");
    }
    #[tokio::test]
    async fn test_get_last_verified_block_hash() {
        let anvil = Anvil::new().fork("https://eth.merkle.io").fork_block_number(20395662).try_spawn().expect("issue while forking");
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider =
            ProviderBuilder::new().on_http(rpc_url.clone());
        let contract = StarknetCoreContract::new(address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4"), provider.clone());

        let eth_client = EthereumClient {
            provider:Arc::new(provider),
            l1_core_contract: contract.clone()
        };
        let block_hash = eth_client.get_last_verified_block_hash().await.expect("issue while getting the last verified block hash");
        let expected = u256_to_starkfelt(U256::from_str_radix("563216050958639290223177746678863910249919294431961492885921903486585884664", 10).unwrap()).unwrap();
        assert_eq!(block_hash, expected, "latest block hash not matching");
    }

    #[tokio::test]
    async fn test_get_last_state_root() {
        let anvil = Anvil::new().fork("https://eth.merkle.io").fork_block_number(20395662).try_spawn().expect("issue while forking");
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider =
            ProviderBuilder::new().on_http(rpc_url.clone());
        let contract = StarknetCoreContract::new(address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4"), provider.clone());

        let eth_client = EthereumClient {
            provider:Arc::new(provider),
            l1_core_contract: contract.clone()
        };
        let state_root = eth_client.get_last_state_root().await.expect("issue while getting the state root");
        let expected = u256_to_starkfelt(U256::from_str_radix("1456190284387746219409791261254265303744585499659352223397867295223408682130", 10).unwrap()).unwrap();
        assert_eq!(state_root, expected, "latest block state root not matching");
    }
    #[tokio::test]
    async fn test_get_last_verified_block_number() {
        // https://etherscan.io/tx/0xcadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81
        // link of the txn, we are using here ^
        let anvil = Anvil::new().fork("https://eth.merkle.io").fork_block_number(20395662).try_spawn().expect("issue while forking");
        let rpc_url: Url = anvil.endpoint().parse().expect("issue while parsing");
        let provider =
            ProviderBuilder::new().on_http(rpc_url.clone());
        let contract = StarknetCoreContract::new(address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4"), provider.clone());

        let eth_client = EthereumClient {
            provider:Arc::new(provider),
            l1_core_contract: contract.clone()
        };

        let block_number = eth_client.get_last_verified_block_number().await.expect("issue");
        assert_eq!(block_number, 662703, "verified block number not matching");
        // eth_client.provider.evm_mine(MineOptions).await.unwrap();
        // let program_output: Vec<U256> = vec![
        //     U256::from_str_radix("385583000215522627239242976168121030194276694037809209719296244325017114533", 10).unwrap(),
        //     U256::from_str_radix("1456190284387746219409791261254265303744585499659352223397867295223408682130", 10).unwrap(),
        //     U256::from_str_radix("662703", 10).unwrap(),
        //     U256::from_str_radix("563216050958639290223177746678863910249919294431961492885921903486585884664", 10).unwrap(),
        //     U256::from_str_radix("2590421891839256512113614983194993186457498815986333310670788206383913888162", 10).unwrap(),
        //     U256::from_str_radix("1", 10).unwrap(),
        //     U256::from_str_radix("3533448494457295048090579982164017552529247335813595504776", 10).unwrap(),
        //     U256::from_str_radix("3354002613483106078443616993383779891315152194217765408699", 10).unwrap(),
        //     U256::from_str_radix("1352442898509484342812941615195522615047870225730820794489190755019968394773", 10).unwrap(),
        //     U256::from_str_radix("153686972708216174382629263233050825733", 10).unwrap(),
        //     U256::from_str_radix("150545786018335208859655177629570777577", 10).unwrap(),
        //     U256::from_str_radix("0", 10).unwrap(),
        //     U256::from_str_radix("20", 10).unwrap(),
        //     U256::from_str_radix("993696174272377493693496825928908586134624850969", 10).unwrap(),
        //     U256::from_str_radix("3256441166037631918262930812410838598500200462657642943867372734773841898370", 10).unwrap(),
        //     U256::from_str_radix("1658082", 10).unwrap(),
        //     U256::from_str_radix("774397379524139446221206168840917193112228400237242521560346153613428128537", 10).unwrap(),
        //     U256::from_str_radix("5", 10).unwrap(),
        //     U256::from_str_radix("4543560", 10).unwrap(),
        //     U256::from_str_radix("876900982330453444151957238745086287996007618046", 10).unwrap(),
        //     U256::from_str_radix("3605988885814994344780002037579309481151562922643941360339142174034211498365", 10).unwrap(),
        //     U256::from_str_radix("8500000000000000000", 10).unwrap(),
        //     U256::from_str_radix("0", 10).unwrap(),
        //     U256::from_str_radix("993696174272377493693496825928908586134624850969", 10).unwrap(),
        //     U256::from_str_radix("3256441166037631918262930812410838598500200462657642943867372734773841898370", 10).unwrap(),
        //     U256::from_str_radix("1658083", 10).unwrap(),
        //     U256::from_str_radix("774397379524139446221206168840917193112228400237242521560346153613428128537", 10).unwrap(),
        //     U256::from_str_radix("5", 10).unwrap(),
        //     U256::from_str_radix("4543560", 10).unwrap(),
        //     U256::from_str_radix("276398225428076927278275581496827486548522150232", 10).unwrap(),
        //     U256::from_str_radix("553080211211254152159931356206975984149148973124973644253545915677805583166", 10).unwrap(),
        //     U256::from_str_radix("20000000000000000", 10).unwrap(),
        //     U256::from_str_radix("0", 10).unwrap(),
        // ];
        // let _ = eth_client.provider.anvil_impersonate_account(address!("2C169DFe5fBbA12957Bdd0Ba47d9CEDbFE260CA7"));
        // let x = hex::decode("b3228e8ba3cb9c397b3ce114decf32fe4a35e380741b2424c910658fe77b967d4e7f5fdc19829ccaba50606b8545e7c0").unwrap();
        // let x: Bytes = Bytes::from(x);
        // let kzg_bytes = Bytes::from("b3228e8ba3cb9c397b3ce114decf32fe4a35e380741b2424c910658fe77b967d4e7f5fdc19829ccaba50606b8545e7c0");
        // let txn = contract.updateStateKzgDA(program_output, x.clone());
        // let tx_hash = txn.send().await.expect("issue while making the call x").watch().await.expect("issue while making the call");
        // let binding = hex::decode("019402299dbda430cef9083b9c57e6666a7eee938cee2c056e3c7dcdb708597b").unwrap();
        // let y = binding.as_slice();
        // let y: Vec<FixedBytes<32>> = vec![FixedBytes::from_slice(y)];
        // let mut tx = txn.into_transaction_request().from(address!("2C169DFe5fBbA12957Bdd0Ba47d9CEDbFE260CA7"));

        // let x = hex::decode("cadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81").unwrap();
        // // let z: BlobTransactionSidecar = BlobTransactionSidecar
        // let already_done = TxHash::from_slice(x.as_slice());
        // let y = (eth_client.provider.get_transaction_by_hash(already_done).await.expect("issue while getting the txn")).unwrap().into_request();
        // tx.blob_versioned_hashes = y.blob_versioned_hashes;
        // tx.transaction_type = y.transaction_type;
        // tx.nonce = y.nonce;
        // tx.chain_id = y.chain_id;
        // tx.access_list = y.access_list;
        // tx.gas = y.gas;
        // tx.gas = y.gas_price;
        // tx.max_fee_per_blob_gas = y.max_fee_per_blob_gas;
        // tx.max_fee_per_gas = y.max_fee_per_gas;
        // tx.max_priority_fee_per_gas = y.max_priority_fee_per_gas;
        // tx.sidecar = y.sidecar;
        // tx.value = y.value;
        // tx.input = y.input;
        // println!(" transaction... {:?}, {:?}", tx.clone(), y.clone());
        // let pending_tx = eth_client.provider.send_transaction(tx).await.expect("issue while sending the txn");
        //

        //
        // // Wait for the transaction to be included and get the receipt.
        // let receipt = pending_tx.get_receipt().await.expect("issue while getting the receipt");
        //
        // println!(
        //     "Transaction included in block {}",
        //     receipt.block_number.expect("Failed to get block number")
        // );
        //
        // println!(
        //     "Transaction receipt is {:?}",
        //     receipt
        // );
        // let number_here = eth_client.get_last_verified_block_number().await.expect("issue");
        //
        // assert_eq!(number_here, 662703, "failing after the call");



        // println!("Deployed contract at address: {}", contract.address());
    }

}
// #[cfg(test)]
// mod l1_sync_tests {
//     use ethers::contract::EthEvent;
//     use ethers::core::types::*;
//     use ethers::prelude::*;
//     use ethers::providers::Provider;
//     use tokio;
//     use url::Url;
//
//     use super::*;
//     use crate::l1::EthereumClient;
//
//     #[derive(Clone, Debug, EthEvent)]
//     pub struct Transfer {
//         #[ethevent(indexed)]
//         pub from: Address,
//         #[ethevent(indexed)]
//         pub to: Address,
//         pub tokens: U256,
//     }
//
//     pub mod eth_rpc {
//         pub const MAINNET: &str = "<ENTER-YOUR-RPC-URL-HERE>";
//     }
//
//     #[tokio::test]
//     #[ignore]
//     async fn test_starting_block() {
//         let url = Url::parse(eth_rpc::MAINNET).expect("Failed to parse URL");
//         let client = EthereumClient::new(url, H160::zero()).await.expect("Failed to create EthereumClient");
//
//         let start_block =
//             EthereumClient::get_last_event_block_number(&client).await.expect("Failed to get last event block number");
//         println!("The latest emission of the LogStateUpdate event was on block: {:?}", start_block);
//     }
//
//     #[tokio::test]
//     #[ignore]
//     async fn test_initial_state() {
//         let url = Url::parse(eth_rpc::MAINNET).expect("Failed to parse URL");
//         let client = EthereumClient::new(url, H160::zero()).await.expect("Failed to create EthereumClient");
//
//         let initial_state = EthereumClient::get_initial_state(&client).await.expect("Failed to get initial state");
//         assert!(!initial_state.global_root.bytes().is_empty(), "Global root should not be empty");
//         assert!(!initial_state.block_number > 0, "Block number should be greater than 0");
//         assert!(!initial_state.block_hash.bytes().is_empty(), "Block hash should not be empty");
//     }
//
//     #[tokio::test]
//     #[ignore]
//     async fn test_event_subscription() -> Result<(), Box<dyn std::error::Error>> {
//         abigen!(
//             IERC20,
//             r#"[
//                 function totalSupply() external view returns (uint256)
//                 function balanceOf(address account) external view returns (uint256)
//                 function transfer(address recipient, uint256 amount) external returns (bool)
//                 function allowance(address owner, address spender) external view returns (uint256)
//                 function approve(address spender, uint256 amount) external returns (bool)
//                 function transferFrom( address sender, address recipient, uint256 amount) external returns (bool)
//                 event Transfer(address indexed from, address indexed to, uint256 value)
//                 event Approval(address indexed owner, address indexed spender, uint256 value)
//             ]"#,
//         );
//
//         const WETH_ADDRESS: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
//
//         let provider = Provider::<Http>::try_from(eth_rpc::MAINNET)?;
//         let client = Arc::new(provider);
//         let address: Address = WETH_ADDRESS.parse()?;
//         let contract = IERC20::new(address, client);
//
//         let event = contract.event::<Transfer>().from_block(0).to_block(EthBlockNumber::Latest);
//
//         let mut event_stream = event.stream().await?;
//
//         while let Some(event_result) = event_stream.next().await {
//             match event_result {
//                 Ok(log) => {
//                     println!("Transfer event: {:?}", log);
//                 }
//                 Err(e) => println!("Error while listening for events: {:?}", e),
//             }
//         }
//
//         Ok(())
//     }
// }
