use alloy::node_bindings::AnvilInstance;
use alloy::providers::{ext::AnvilApi, ProviderBuilder};
use alloy::{node_bindings::Anvil, sol};
use alloy_primitives::Address;
use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use utils::env_utils::get_env_var_or_panic;

// Using the Pipe trait to write chained operations easier
#[allow(dead_code)]
trait Pipe: Sized {
    fn pipe<T, F: FnOnce(Self) -> T>(self, f: F) -> T {
        f(self)
    }
}

// Implement Pipe for all types
impl<S> Pipe for S {}

// TODO: betterment of file routes

use lazy_static::lazy_static;
use url::Url;

lazy_static! {
    static ref ENV_FILE_PATH: PathBuf = PathBuf::from(".env.test");
    static ref CURRENT_PATH: String = env::current_dir()
        .expect("Failed to get current directory")
        .to_str()
        .expect("Path contains invalid Unicode")
        .to_string();
    static ref ETH_RPC: String = get_env_var_or_panic("ETHEREUM_BLAST_RPC_URL");
    pub static ref STARKNET_OPERATOR_ADDRESS: Address =
        Address::from_str("0x2C169DFe5fBbA12957Bdd0Ba47d9CEDbFE260CA7").expect("Unable to parse address");
    static ref STARKNET_CORE_CONTRACT_ADDRESS: Address =
        Address::from_str("0xc662c410c0ecf747543f5ba90660f6abebd9c8c4").expect("Could not impersonate account.");
    pub static ref TEST_NONCE: u64 = 666068;
}

// SOLIDITY FUNCTIONS NEEDED
sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    STARKNET_CORE_CONTRACT,
    "src/test_data/contract_abi/starknet_core_contract.json"
);

sol! {
    #[allow(missing_docs)]
    #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506101c18061001c5f395ff3fe608060405234801561000f575f80fd5b5060043610610029575f3560e01c8063b72d42a11461002d575b5f80fd5b6100476004803603810190610042919061010d565b610049565b005b50505050565b5f80fd5b5f80fd5b5f80fd5b5f80fd5b5f80fd5b5f8083601f84011261007857610077610057565b5b8235905067ffffffffffffffff8111156100955761009461005b565b5b6020830191508360208202830111156100b1576100b061005f565b5b9250929050565b5f8083601f8401126100cd576100cc610057565b5b8235905067ffffffffffffffff8111156100ea576100e961005b565b5b6020830191508360018202830111156101065761010561005f565b5b9250929050565b5f805f80604085870312156101255761012461004f565b5b5f85013567ffffffffffffffff81111561014257610141610053565b5b61014e87828801610063565b9450945050602085013567ffffffffffffffff81111561017157610170610053565b5b61017d878288016100b8565b92509250509295919450925056fea2646970667358221220fa7488d5a2a9e6c21e6f46145a831b0f04fdebab83868dc2b996c17f8cba4d8064736f6c634300081a0033")]
    contract DummyCoreContract {
        function updateStateKzgDA(uint256[] calldata programOutput, bytes calldata kzgProof)  external {
        }
    }
}

pub struct EthereumTestBuilder {
    fork_block: Option<u64>,
    impersonator: Option<Address>,
}

#[allow(dead_code)]
pub struct EthereumTest {
    _anvil: AnvilInstance,
    provider: alloy::providers::RootProvider<alloy::transports::http::Http<reqwest::Client>>,
    pub rpc_url: Url,
}

#[allow(clippy::new_without_default)]
impl EthereumTestBuilder {
    pub fn new() -> Self {
        EthereumTestBuilder { fork_block: None, impersonator: None }
    }

    pub fn with_fork_block(mut self, block_no: u64) -> Self {
        self.fork_block = Some(block_no);
        self
    }

    pub fn with_impersonator(mut self, impersonator: Address) -> Self {
        self.impersonator = Some(impersonator);
        self
    }

    pub async fn build(&self) -> EthereumTest {
        // Load ENV vars
        dotenvy::from_filename(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

        // Setup Anvil
        let anvil = match self.fork_block {
            Some(fork_block) => {
                Anvil::new().fork(&*ETH_RPC).fork_block_number(fork_block).try_spawn().expect("Could not spawn Anvil.")
            }
            None => Anvil::new().try_spawn().expect("Could not spawn Anvil."),
        };

        // Setup Provider
        let provider = ProviderBuilder::new().on_http(anvil.endpoint_url());

        if let Some(impersonator) = self.impersonator {
            provider.anvil_impersonate_account(impersonator).await.expect("Unable to impersonate account.");
        }

        let rpc_url = anvil.endpoint_url();

        EthereumTest { _anvil: anvil, provider, rpc_url }
    }
}

#[cfg(test)]
#[cfg(feature = "testing")]
mod settlement_client_tests {
    use crate::conversion::to_padded_hex;
    use crate::tests::{
        DummyCoreContract, EthereumTestBuilder, Pipe, CURRENT_PATH, STARKNET_CORE_CONTRACT,
        STARKNET_CORE_CONTRACT_ADDRESS, STARKNET_OPERATOR_ADDRESS,
    };
    use crate::EthereumSettlementClient;
    use alloy::eips::eip4844::BYTES_PER_BLOB;
    use alloy::providers::Provider;
    use alloy::sol_types::private::U256;
    use alloy_primitives::FixedBytes;
    use color_eyre::eyre::eyre;
    use rstest::rstest;
    use settlement_client_interface::{SettlementClient, SettlementVerificationStatus};
    use std::fs;
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::str::FromStr;
    use std::time::Duration;
    use tokio::time::sleep;

    #[rstest]
    #[tokio::test]
    /// Tests if the method is able to do a transaction with same function selector on a dummy contract.
    /// If we impersonate starknet operator then we loose out on testing for validity of signature in the transaction.
    /// Starknet core contract has a modifier `onlyOperator` that restricts anyone but the operator to send transaction to `updateStateKzgDa` method
    /// And hence to test the signature and transaction via a dummy contract that has same function selector as `updateStateKzgDa`.
    /// and anvil is for testing on fork Eth.
    async fn update_state_blob_with_dummy_contract_works() {
        let setup = EthereumTestBuilder::new().build().await;

        // Deploying a dummy contract
        let contract = DummyCoreContract::deploy(&setup.provider).await.expect("Unable to deploy address");
        let ethereum_settlement_client = EthereumSettlementClient::with_test_settings(
            setup.provider.clone(),
            *contract.address(),
            setup.rpc_url,
            None,
        );

        // Getting latest nonce after deployment
        let nonce = ethereum_settlement_client.get_nonce().await.expect("Unable to fetch nonce");

        // keeping 9 elements because the code accesses 8th index as program output
        let program_output = vec![[0; 32]; 9];
        // keeping one element as we've a check in build_proof
        let blob_data_vec = vec![vec![0; BYTES_PER_BLOB]];

        // Calling update_state_with_blobs
        let update_state_result = ethereum_settlement_client
            .update_state_with_blobs(program_output, blob_data_vec, nonce)
            .await
            .expect("Could not go through update_state_with_blobs.");

        // Asserting, Expected to receive transaction hash.
        assert!(!update_state_result.is_empty(), "No transaction Hash received.");

        let txn = setup
            .provider
            .get_transaction_by_hash(FixedBytes::from_str(update_state_result.as_str()).expect("Unable to convert txn"))
            .await
            .expect("did not get txn from hash")
            .unwrap();

        assert_eq!(txn.hash.to_string(), update_state_result.to_string());
        assert!(txn.signature.is_some());
        assert_eq!(txn.to.unwrap(), *contract.address());

        // Testing verify_tx_inclusion
        sleep(Duration::from_secs(2)).await;
        ethereum_settlement_client
            .wait_for_tx_finality(update_state_result.as_str())
            .await
            .expect("Could not wait for txn finality.");
        let verified_inclusion = ethereum_settlement_client
            .verify_tx_inclusion(update_state_result.as_str())
            .await
            .expect("Could not verify inclusion.");
        assert_eq!(verified_inclusion, SettlementVerificationStatus::Verified);
    }

    #[rstest]
    #[tokio::test]
    #[case::basic(20468827)]
    /// tests if the method is able to impersonate the`Starknet Operator` and do an `update_state` transaction.
    /// We impersonate the Starknet Operator to send a transaction to the Core contract
    /// Here signature checks are bypassed and anvil is for testing on fork Eth.
    async fn update_state_blob_with_impersonation_works(#[case] fork_block_no: u64) {
        let setup = EthereumTestBuilder::new()
            .with_fork_block(fork_block_no)
            .with_impersonator(*STARKNET_OPERATOR_ADDRESS)
            .build()
            .await;
        let ethereum_settlement_client = EthereumSettlementClient::with_test_settings(
            setup.provider.clone(),
            *STARKNET_CORE_CONTRACT_ADDRESS,
            setup.rpc_url,
            Some(*STARKNET_OPERATOR_ADDRESS),
        );

        // let nonce = ethereum_settlement_client.get_nonce().await.expect("Unable to fetch nonce");

        let nonce = setup
            .provider
            .get_transaction_count(*STARKNET_OPERATOR_ADDRESS)
            .await
            .unwrap()
            .to_string()
            .parse::<u64>()
            .unwrap();
        // Create a contract instance.
        let contract = STARKNET_CORE_CONTRACT::new(*STARKNET_CORE_CONTRACT_ADDRESS, setup.provider.clone());

        // Call the contract, retrieve the current stateBlockNumber.
        let prev_block_number = contract.stateBlockNumber().call().await.unwrap();

        // generating program output and blob vector
        let program_output = get_program_output(fork_block_no + 1);
        let blob_data_vec = get_blob_data(fork_block_no + 1);

        // Calling update_state_with_blobs
        let update_state_result = ethereum_settlement_client
            .update_state_with_blobs(program_output, blob_data_vec, nonce)
            .await
            .expect("Could not go through update_state_with_blobs.");

        // Asserting, Expected to receive transaction hash.
        assert!(!update_state_result.is_empty(), "No transaction Hash received.");

        sleep(Duration::from_secs(5)).await;
        ethereum_settlement_client
            .wait_for_tx_finality(update_state_result.as_str())
            .await
            .expect("Could not wait for txn finality.");

        let verified_inclusion = ethereum_settlement_client
            .verify_tx_inclusion(update_state_result.as_str())
            .await
            .expect("Could not verify inclusion.");
        assert_eq!(verified_inclusion, SettlementVerificationStatus::Verified);

        // Call the contract, retrieve the latest stateBlockNumber.
        let latest_block_number = contract.stateBlockNumber().call().await.unwrap();

        assert_eq!(prev_block_number._0.as_u32() + 1, latest_block_number._0.as_u32());
    }

    #[rstest]
    #[tokio::test]
    #[case::typical(20468827)]
    async fn get_last_settled_block_typical_works(#[case] fork_block_no: u64) {
        let setup = EthereumTestBuilder::new().with_fork_block(fork_block_no).build().await;
        let ethereum_settlement_client = EthereumSettlementClient::with_test_settings(
            setup.provider.clone(),
            *STARKNET_CORE_CONTRACT_ADDRESS,
            setup.rpc_url,
            None,
        );
        assert_eq!(
            ethereum_settlement_client.get_last_settled_block().await.expect("Could not get last settled block."),
            666039
        );
    }

    #[rstest]
    #[tokio::test]
    #[case::basic(20468828)]
    async fn creating_input_data_works(#[case] fork_block_no: u64) {
        use c_kzg::Bytes32;

        use crate::conversion::get_input_data_for_eip_4844;

        let program_output = get_program_output(fork_block_no);
        let blob_data_vec = get_blob_data(fork_block_no);

        let x_0_value_bytes32 = Bytes32::from(program_output[8]);

        let kzg_proof = EthereumSettlementClient::build_proof(blob_data_vec, x_0_value_bytes32)
            .expect("Unable to build KZG proof for given params.")
            .to_owned();

        let input_bytes = get_input_data_for_eip_4844(program_output, kzg_proof).expect("unable to create input data");
        let expected = "0xb72d42a100000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000340000000000000000000000000000000000000000000000000000000000000001706ac7b2661801b4c0733da6ed1d2910b3b97259534ca95a63940932513111fba028bccc051eaae1b9a69b53e64a68021233b4dee2030aeda4be886324b3fbb3e00000000000000000000000000000000000000000000000000000000000a29b8070626a88de6a77855ecd683757207cdd18ba56553dca6c0c98ec523b827bee005ba2078240f1585f96424c2d1ee48211da3b3f9177bf2b9880b4fc91d59e9a2000000000000000000000000000000000000000000000000000000000000000100000000000000002b4e335bc41dc46c71f29928a5094a8c96a0c3536cabe53e0000000000000000810abb1929a0d45cdd62a20f9ccfd5807502334e7deb35d404c86d8b63a5741770fefca2f9b8efb7e663d89097edb3c60595b236f6e78e6f000000000000000000000000000000004a4b8a979fefc4d6b82e030fb082ca98000000000000000000000000000000004e8371c6774260e87b92447d4a2b0e170000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000bf67f59d2988a46fbff7ed79a621778a3cd3985b0088eedbe2fe3918b69ccb411713b7fa72079d4eddf291103ccbe41e78a9615c0000000000000000000000000000000000000000000000000000000000194fe601b64b1b3b690b43b9b514fb81377518f4039cd3e4f4914d8a6bdf01d679fb1900000000000000000000000000000000000000000000000000000000000000050000000000000000000000007f39c581f595b53c5cb19bd0b3f8da6c935e2ca000000000000000000000000012ccc443d39da45e5f640b3e71f0c7502152dbac01d4988e248d342439aa025b302e1f07595f6a5c810dcce23e7379e48f05d4cf000000000000000000000000000000000000000000000007f189b5374ad2a00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000030ab015987628cffee3ef99b9768ef8ca12c6244525f0cd10310046eaa21291b5aca164d044c5b4ad7212c767b165ed5e300000000000000000000000000000000";
        assert_eq!(input_bytes, expected);
    }

    // UTILITY FUNCTIONS NEEDED

    fn get_program_output(block_no: u64) -> Vec<[u8; 32]> {
        // Program Output
        let program_output_file_path =
            format!("{}{}{}{}", *CURRENT_PATH, "/src/test_data/program_output/", block_no, ".txt");

        let mut program_output: Vec<[u8; 32]> = Vec::new();
        let file = File::open(program_output_file_path).expect("Failed to read program output file");
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line.expect("can't read line");
            let trimmed = line.trim();
            assert!(!trimmed.is_empty());

            let result: [u8; 32] = U256::from_str(trimmed)
                .expect("Unable to convert line")
                .to_be_bytes_vec()
                .as_slice()
                .pipe(to_padded_hex)
                .pipe(|hex| hex_string_to_u8_vec(&hex).expect("unable to convert"))
                .try_into()
                .expect("Vector length must be 32");

            program_output.push(result)
        }
        program_output
    }

    fn get_blob_data(block_no: u64) -> Vec<Vec<u8>> {
        // Blob Data
        let blob_data_file_path = format!("{}{}{}{}", *CURRENT_PATH, "/src/test_data/blob_data/", block_no, ".txt");
        let blob_data = fs::read_to_string(blob_data_file_path).expect("Failed to read the blob data txt file");
        let blob_data_vec = vec![hex_string_to_u8_vec(&blob_data).unwrap()];
        blob_data_vec
    }

    fn hex_string_to_u8_vec(hex_str: &str) -> color_eyre::Result<Vec<u8>> {
        // Remove any spaces or non-hex characters from the input string
        let cleaned_str: String = hex_str.chars().filter(|c| c.is_ascii_hexdigit()).collect();

        // Convert the cleaned hex string to a Vec<u8>
        let mut result = Vec::new();
        for chunk in cleaned_str.as_bytes().chunks(2) {
            if let Ok(byte_val) = u8::from_str_radix(std::str::from_utf8(chunk)?, 16) {
                result.push(byte_val);
            } else {
                return Err(eyre!("Error parsing hex string: {}", cleaned_str));
            }
        }

        Ok(result)
    }
}
