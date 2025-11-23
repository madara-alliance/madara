use std::str::FromStr;
use std::time::Duration;

use alloy::dyn_abi::SolType;
use alloy::network::EthereumWallet;
use alloy::primitives::{fixed_bytes, Address, Bytes, I256, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use orchestrator_utils::env_utils::get_env_var_or_panic;
use tokio::time::sleep;
use url::Url;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    StarknetCoreContract,
    "artifacts/contracts/Starknet.json"
);

sol! {
    struct StateUpdate {
        uint256 globalRoot;
        int256 blockNumber;
        uint256 blockHash;
    }

    struct InitializeData {
        uint256 programHash;
        uint256 aggregatorProgramHash;
        address verifier;
        uint256 configHash;
        StateUpdate initialState;
    }
}

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    GPSVerifier,
    "artifacts/contracts/GPSVerifier.json"
);

pub struct AnvilSetup {
    pub rpc_url: Url,
}

impl AnvilSetup {
    pub fn new() -> Self {
        let rpc_url = Url::from_str("http://localhost:8545").unwrap();
        Self { rpc_url }
    }

    pub async fn deploy_contracts(&self) -> (Address, Address) {
        let wallet = EthereumWallet::from(
            PrivateKeySigner::from_str(&get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY")).unwrap(),
        );
        let provider =
            ProviderBuilder::new().disable_recommended_fillers().wallet(wallet).connect_http(self.rpc_url.clone());

        let starknet_core_contract_client = StarknetCoreContract::deploy(&provider).await.unwrap();
        log::info!("📦 Deployed starknet_core_contract at address: {}", starknet_core_contract_client.address());
        let verifier_client = GPSVerifier::deploy(&provider).await.unwrap();

        // This is the fact hash calculated from get_fact_info() or mongodb job metadata
        // for aggregator program ran on blocks 309146 to 309148 (both included)
        // One such txn on Ethereum Sepolia can be found here
        // https://sepolia.etherscan.io/tx/0x51a6e4e76f9426abdd0bd19cfcea195932772e77183669ce6ed1037298ddc464
        let fact_hash = fixed_bytes!("b39f2de70bdf515138173a43ebc783273e5b2329d7325e887fbd1c0b1cac53a4");
        let _ = verifier_client.setValid(fact_hash).send().await.expect("Failed to set fact as valid");
        sleep(Duration::from_secs(10)).await;
        let _is_fact_valid = verifier_client.isValid(fact_hash).call().await.unwrap();
        assert!(_is_fact_valid, "Fact should be valid");
        log::debug!("Is fact valid? {:?}", _is_fact_valid);

        log::debug!("📦 Deployed verifier at address: {}", verifier_client.address());

        let init_data = InitializeData {
            programHash: U256::from_str_radix(
                "2397984267054479079853548842566103781972463965746662494980785692480538410509",
                10,
            )
            .unwrap(),
            aggregatorProgramHash: U256::from_str_radix(
                "15787695375210609250491147414005894154890873413229882671403677761527504080",
                10,
            )
            .unwrap(),
            verifier: *verifier_client.address(),
            configHash: U256::from_str_radix(
                "8868593919264901768958912247765226517850727970326290266005120699201631282",
                10,
            )
            .unwrap(),
            initialState: StateUpdate {
                globalRoot: U256::from_str_radix(
                    "909296745053732661287194258973489917070052674873164458444078526806377759304",
                    10,
                )
                .unwrap(),
                blockNumber: I256::from_dec_str("309145").unwrap(),
                blockHash: U256::from_str_radix(
                    "3220418376726891625345370796158843447968358635074716858170481552624095325195",
                    10,
                )
                .unwrap(),
            },
        };

        let encoded_data = InitializeData::abi_encode(&init_data);
        let builder = starknet_core_contract_client.initializeContractState(Bytes::from(encoded_data));
        let tx_hash = builder.send().await.unwrap().watch().await.unwrap();
        println!("📦 Contract setup done. Txn Hash : {}", tx_hash);
        (*starknet_core_contract_client.address(), *verifier_client.address())
    }
}

impl Default for AnvilSetup {
    fn default() -> Self {
        Self::new()
    }
}
