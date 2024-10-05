use std::str::FromStr;

use alloy::dyn_abi::SolType;
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, I256, U256};
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use url::Url;
use utils::env_utils::get_env_var_or_panic;

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
        let wallet =
            EthereumWallet::from(PrivateKeySigner::from_str(&get_env_var_or_panic("ETHEREUM_PRIVATE_KEY")).unwrap());
        let provider = ProviderBuilder::new().with_recommended_fillers().wallet(wallet).on_http(self.rpc_url.clone());

        let starknet_core_contract_client = StarknetCoreContract::deploy(&provider).await.unwrap();
        println!("📦 Deployed starknet_core_contract at address: {}", starknet_core_contract_client.address());

        let verifier_client = GPSVerifier::deploy(&provider).await.unwrap();
        println!("📦 Deployed verifier at address: {}", verifier_client.address());

        let init_data = InitializeData {
            programHash: U256::from(0u64),
            aggregatorProgramHash: U256::from(0u64),
            verifier: *verifier_client.address(),
            configHash: U256::from_str_radix(
                "8868593919264901768958912247765226517850727970326290266005120699201631282",
                10,
            )
            .unwrap(),
            initialState: StateUpdate {
                globalRoot: U256::from_str_radix(
                    "1013056298804193110052941398049455744613709243892509969624981484583467797561",
                    10,
                )
                .unwrap(),
                blockNumber: I256::from_dec_str("66644").unwrap(),
                blockHash: U256::from_str_radix(
                    "864771394108447903429963973655367092508118282522891417641831341021946765687",
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
