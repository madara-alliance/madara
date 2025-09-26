use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use alloy::node_bindings::{Anvil, AnvilInstance};
use alloy::primitives::Address;
use alloy::providers::ext::AnvilApi;
use alloy::providers::ProviderBuilder;
use alloy::sol;
use orchestrator_utils::env_utils::get_env_var_or_panic;
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
    static ref ETH_RPC: String = get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL");
    pub static ref MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS: Address =
        Address::from_str(get_env_var_or_panic("MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS").as_str())
            .expect("Could not parse MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS");
    static ref STARKNET_CORE_CONTRACT_ADDRESS: Address =
        Address::from_str(get_env_var_or_panic("MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS").as_str())
            .expect("Could not parse MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS");
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
    #[sol(rpc, bytecode="6080604052348015600e575f80fd5b506101c18061001c5f395ff3fe608060405234801561000f575f80fd5b5060043610610029575f3560e01c8063507ee5281461002d575b5f80fd5b6100476004803603810190610042919061010d565b610049565b005b50505050565b5f80fd5b5f80fd5b5f80fd5b5f80fd5b5f80fd5b5f8083601f84011261007857610077610057565b5b8235905067ffffffffffffffff8111156100955761009461005b565b5b6020830191508360208202830111156100b1576100b061005f565b5b9250929050565b5f8083601f8401126100cd576100cc610057565b5b8235905067ffffffffffffffff8111156100ea576100e961005b565b5b6020830191508360208202830111156101065761010561005f565b5b9250929050565b5f805f80604085870312156101255761012461004f565b5b5f85013567ffffffffffffffff81111561014257610141610053565b5b61014e87828801610063565b9450945050602085013567ffffffffffffffff81111561017157610170610053565b5b61017d878288016100b8565b92509250509295919450925056fea2646970667358221220a4f885f02f3fe00b96deaedfe0b727380694ef82ad021223472395e757405c1b64736f6c634300081a0033")]
    contract DummyCoreContract {
        function updateStateKzgDA(uint256[] calldata programOutput, bytes[] calldata kzgProof)  external {
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

const BLOCK_TIME: u64 = 6;

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
        dotenvy::from_filename_override(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

        // Setup Anvil
        let anvil = match self.fork_block {
            Some(fork_block) => Anvil::new()
                .fork(&*ETH_RPC)
                .fork_block_number(fork_block)
                .block_time(BLOCK_TIME)
                .try_spawn()
                .expect("Could not spawn Anvil."),
            None => Anvil::new().block_time(BLOCK_TIME).try_spawn().expect("Could not spawn Anvil."),
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
    use std::fs;
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::str::FromStr;
    use std::time::Duration;

    use alloy::consensus::Transaction;
    use alloy::eips::eip4844::BYTES_PER_BLOB;
    use alloy::primitives::Address;
    use alloy::providers::Provider;
    use alloy::sol_types::private::U256;
    use alloy_primitives::B256;
    use orchestrator_settlement_client_interface::{SettlementClient, SettlementVerificationStatus};
    use orchestrator_utils::env_utils::get_env_var_or_panic;
    use rstest::rstest;
    use tokio::time::sleep;

    use super::{BLOCK_TIME, ENV_FILE_PATH};
    use crate::conversion::{hex_string_to_u8_vec, to_padded_hex};
    use crate::tests::{
        DummyCoreContract, EthereumTestBuilder, Pipe, CURRENT_PATH, MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS,
        STARKNET_CORE_CONTRACT, STARKNET_CORE_CONTRACT_ADDRESS,
    };
    use crate::{EthereumSettlementClient, EthereumSettlementValidatedArgs, N_BLOBS_OFFSET};

    use orchestrator_utils::test_utils::setup_test_data;

    fn get_program_output_from_file(file_path: &str) -> Result<Vec<[u8; 32]>, Box<dyn std::error::Error>> {
        let file = File::open(file_path)?;
        let reader = BufReader::new(file);

        reader
            .lines()
            .filter_map(|line| match line {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        let mut buf = [0u8; 32];
                        match hex::decode_to_slice(trimmed, &mut buf) {
                            Ok(_) => Some(Ok(buf)),
                            Err(e) => Some(Err(e.into())),
                        }
                    }
                }
                Err(e) => Some(Err(e.into())),
            })
            .collect()
    }

    fn get_blob_data_from_file(file_path: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(file_path)?;
        let bytes = hex_string_to_u8_vec(&content)?;
        Ok(bytes)
    }

    #[rstest]
    #[tokio::test]
    /// Tests if the method is able to do a transaction with the same function selector on a dummy
    /// contract. If we impersonate a starknet operator, then we lose out on testing for validity
    /// of signature in the transaction. Starknet core contract has a modifier `onlyOperator`
    /// that restricts anyone but the operator to send transaction to `updateStateKzgDa` method
    /// And hence to test the signature and transaction via a dummy contract that has the same
    /// function selector as `updateStateKzgDa`. Anvil is for testing on fork Eth.
    async fn update_state_blob_with_dummy_contract_works() {
        dotenvy::from_filename_override(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

        let setup = EthereumTestBuilder::new().build().await;

        let ethereum_settlement_params = EthereumSettlementValidatedArgs {
            ethereum_rpc_url: setup.rpc_url,
            ethereum_private_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY"),
            l1_core_contract_address: Address::from_str(&get_env_var_or_panic(
                "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS",
            ))
            .expect("Invalid L1 core contract address"),
            starknet_operator_address: Address::from_str(&get_env_var_or_panic(
                "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS",
            ))
            .expect("Invalid Starknet operator address"),
            txn_wait_sleep_delay_secs: 10,
        };

        // Deploying a dummy contract
        let contract = DummyCoreContract::deploy(&setup.provider).await.expect("Unable to deploy address");
        let ethereum_settlement_client = EthereumSettlementClient::with_test_params(
            setup.provider.clone(),
            *contract.address(),
            ethereum_settlement_params.ethereum_rpc_url,
            None,
        );

        // Getting latest nonce after deployment
        let nonce = ethereum_settlement_client.get_nonce().await.expect("Unable to fetch nonce");

        // keeping 16 elements because the code accesses 10th index as program output
        let mut program_output = vec![[0; 32]; 16];
        program_output[N_BLOBS_OFFSET][31] = 1; // number of blobs == 1

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
            .get_transaction_by_hash(B256::from_str(update_state_result.as_str()).expect("Unable to convert txn"))
            .await
            .expect("did not get txn from hash")
            .unwrap();

        assert_eq!(txn.inner.tx_hash().to_string(), update_state_result.to_string());
        assert!(!txn.inner.signature().as_bytes().is_empty());
        assert_eq!(txn.inner.to().unwrap(), *contract.address());

        // Testing verify_tx_inclusion
        sleep(Duration::from_secs(BLOCK_TIME + 2)).await;
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
    #[case::basic(6806847)]
    /// tests if the method is able to impersonate the`Starknet Operator` and do an `update_state`
    /// transaction. We impersonate the Starknet Operator to send a transaction to the Core
    /// contract Here signature checks are bypassed and anvil is for testing on fork Eth.
    async fn update_state_blob_with_impersonation_works(#[case] fork_block_no: u64) {
        dotenvy::from_filename_override(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

        let setup = EthereumTestBuilder::new()
            .with_fork_block(fork_block_no)
            .with_impersonator(*MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS)
            .build()
            .await;

        let ethereum_settlement_params = EthereumSettlementValidatedArgs {
            ethereum_rpc_url: setup.rpc_url,
            ethereum_private_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY"),
            l1_core_contract_address: Address::from_str(&get_env_var_or_panic(
                "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS",
            ))
            .expect("Invalid L1 core contract address"),
            starknet_operator_address: Address::from_str(&get_env_var_or_panic(
                "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS",
            ))
            .expect("Invalid Starknet operator address"),
            txn_wait_sleep_delay_secs: 60u64,
        };

        let ethereum_settlement_client = EthereumSettlementClient::with_test_params(
            setup.provider.clone(),
            ethereum_settlement_params.l1_core_contract_address,
            ethereum_settlement_params.ethereum_rpc_url,
            Some(ethereum_settlement_params.starknet_operator_address),
        );

        // let nonce = ethereum_settlement_client.get_nonce().await.expect("Unable to fetch nonce");

        let nonce = setup
            .provider
            .get_transaction_count(*MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS)
            .await
            .unwrap()
            .to_string()
            .parse::<u64>()
            .unwrap();
        // Create a contract instance.
        let contract = STARKNET_CORE_CONTRACT::new(*STARKNET_CORE_CONTRACT_ADDRESS, setup.provider.clone());

        // generating program output and blob vector
        let program_output = get_program_output(fork_block_no + 1);
        let blob_data_vec = get_blob_data(fork_block_no + 1);

        // Calling update_state_with_blobs
        let update_state_result = ethereum_settlement_client
            .update_state_with_blobs(program_output.clone(), blob_data_vec, nonce)
            .await
            .expect("Could not go through update_state_with_blobs.");

        // Asserting, Expected to receive transaction hash.
        assert!(!update_state_result.is_empty(), "No transaction Hash received.");

        sleep(Duration::from_secs(10)).await;
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

        let expected_latest_block_number = bytes_to_u32(program_output[3].as_slice()).unwrap();
        assert_eq!(expected_latest_block_number, latest_block_number._0.as_u32());
    }

    #[rstest]
    #[tokio::test]
    #[case::typical(6806847)]
    async fn get_last_settled_block_typical_works(#[case] fork_block_no: u64) {
        dotenvy::from_filename_override(&*ENV_FILE_PATH).expect("Could not load .env.test file.");
        let setup = EthereumTestBuilder::new().with_fork_block(fork_block_no).build().await;

        let ethereum_settlement_params = EthereumSettlementValidatedArgs {
            ethereum_rpc_url: setup.rpc_url,
            ethereum_private_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY"),
            l1_core_contract_address: Address::from_str(&get_env_var_or_panic(
                "MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS",
            ))
            .expect("Invalid L1 core contract address"),
            starknet_operator_address: Address::from_str(&get_env_var_or_panic(
                "MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS",
            ))
            .expect("Invalid Starknet operator address"),
            txn_wait_sleep_delay_secs: 60u64,
        };

        let ethereum_settlement_client = EthereumSettlementClient::with_test_params(
            setup.provider.clone(),
            ethereum_settlement_params.l1_core_contract_address,
            ethereum_settlement_params.ethereum_rpc_url,
            None,
        );
        assert_eq!(
            ethereum_settlement_client.get_last_settled_block().await.expect("Could not get last settled block."),
            Some(218378)
        );
    }

    #[rstest]
    #[tokio::test]
    // Ethereum block for Starknet blocks 789878-790377, version 0.13.5, created on 21/05/2025
    // Ethereum Block - https://sepolia.etherscan.io/block/8373665
    // Starknet L1 Transaction - https://sepolia.voyager.online/l1/tx/0x09ad9c188b1abb9f27509eaa0c12aada6b3f446b70319da6d555b49af8cf2c8e
    #[case::basic(8373665)]
    async fn creating_input_data_works(#[case] _fork_block_no: u64) {
        dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");
        // Download test data
        // The test data contains state update information about block 8373665 on Ethereum Sepolia
        // Contains the following files:
        // blobs/1.txt
        // blobs/2.txt
        // program_output.txt
        // ... other files
        let data_dir = setup_test_data(vec![("8373665.tar.gz", true)]).await.expect("unable to get test data");

        let program_output_path = data_dir.path().join("8373665/program_output.txt").to_str().unwrap().to_string();

        // get program output from the file
        let program_output = get_program_output_from_file(&program_output_path.to_string())
            .expect("unable to get program output from the file");

        // get blob data from the file
        let blob_data_vec = vec![
            get_blob_data_from_file(data_dir.path().join("8373665/blobs/1.txt").to_str().unwrap())
                .expect("unable to get blob data from the file"),
            get_blob_data_from_file(data_dir.path().join("8373665/blobs/2.txt").to_str().unwrap())
                .expect("unable to get blob data from the file"),
        ];

        // build input bytes
        let input_bytes = EthereumSettlementClient::build_input_bytes(program_output, blob_data_vec)
            .await
            .expect("unable to build input bytes");
        // expected input bytes
        let expected = "0x507ee52800000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000e60000000000000000000000000000000000000000000000000000000000000007000337cffbb1a0085c53bc0c525f89dc58f49c9d0f387a010bc507370ea52600f047cdceeca2161a96dc8503ce848e93467175566620b0ad9d0ba74191a737ebf00000000000000000000000000000000000000000000000000000000000c0d7500000000000000000000000000000000000000000000000000000000000c0f6905b8101eb9a1bfaabc0f289f6ee21a6880d18b690b3d3d15528da0ab8c267499017db5e18a00f573c66101ef280e0207ed77ce30c8ccc31c33a9714381a123c7059ab9079877a6b3ea0dee995aa259b623ed3952bafaa39c47edc5c257efdc0c000504fa6e5eb930c0d8329d4a77d98391f2730dab8516600aeaf733a61234320000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000006bafe057a8b7146d344c5e0c31395383125fe7ca1f1939694ba6ae27610e8ed00000000000000000000000000000000000000000000000000000000000000020000000000000000919045a069f3fa7c38e26b6926341c85e0262a065ba757530000000000000000b52c3a39c121a8eedbc474b22cd2645b7e3383c42be91cb800000000000000001bcf558e296dfa6246909d8ec4e31f56379df5c88a098cb30000000000000000858f9591fbe243ad2112dbb73e42a3380c5fec20ba18fc5b00000000000000000000000000000000260b28a1b1113eba91c7938c3eb4c9bd000000000000000000000000000000005dfa86d081188199c0861585f4f82f65000000000000000000000000000000000baab1749bde239c68e04c21febb397800000000000000000000000000000000455b2bfc50407f1c46fd217b27b676be000000000000000000000000000000000000000000000000000000000000005a07b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf000000000000000000000000d4be2bd0e6813aaefb6bc35b033f352516b4bed30000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000022e000000000000000000000000000000000000000000000000000000000000019107b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf000000000000000000000000d9db61d3dfccff928f6688fce2efd1852e8aff180000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000033d00000000000000000000000000000000000000000000000000000000000002d107b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf00000000000000000000000001bd71e3dc3a852bebe1fcaadf024b9fbeb71f090000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000000000240069b0b8db5469b82390505d02cda4ca08bebf50ca0d824a88b6756fcd5aaf5e5000000000000000000000000bfdbdba902968241163cdb95f90d5ad2f372c737000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000002bf069b0b8db5469b82390505d02cda4ca08bebf50ca0d824a88b6756fcd5aaf5e5000000000000000000000000a3eac0ffe64d474da60b75f5a0d5aad74bdf1be90000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000035600000000000000000000000000000000000000000000000000000000000001db07b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf00000000000000000000000011d64c9fb3d29faef624438efca65fdae5bdb90e000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000002970000000000000000000000000000000000000000000000000000000000000073069b0b8db5469b82390505d02cda4ca08bebf50ca0d824a88b6756fcd5aaf5e5000000000000000000000000faa956c1adeebb94b5ceffeeacc0a542ed0af3800000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000037b00000000000000000000000000000000000000000000000000000000000002b907b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf0000000000000000000000003ea4d548cb45c02c62e8cbc8b16ef63ec54f3f02000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000003e8000000000000000000000000000000000000000000000000000000000000012807b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf000000000000000000000000d65b8359dcdad1df99ec1d4eddd9b9f3df5aa5ae000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000e400000000000000000000000000000000000000000000000000000000000001b207b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf00000000000000000000000069b8cae7fc79ab1fefda110b3a66fc57d7cabba40000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000017700000000000000000000000000000000000000000000000000000000000000b1069b0b8db5469b82390505d02cda4ca08bebf50ca0d824a88b6756fcd5aaf5e5000000000000000000000000ce1a4ff89033dbcff7abcacddf1b6d2f1eac9f8f0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000002e000000000000000000000000000000000000000000000000000000000000008907b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf0000000000000000000000003bed1dce9bf7f005829ef8cf4cb9e8c0f9a8c4dc000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000aa000000000000000000000000000000000000000000000000000000000000025107b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf0000000000000000000000002c2eaec3c3adb73ae4ff3a9fa5413d6b18cb14d1000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000003870000000000000000000000000000000000000000000000000000000000000125069b0b8db5469b82390505d02cda4ca08bebf50ca0d824a88b6756fcd5aaf5e5000000000000000000000000b4d7c587097ddda1defecdbdc77ca5aabde9a0aa0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000020400000000000000000000000000000000000000000000000000000000000003f807b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf0000000000000000000000007eca5028ee2f15bb5f25fdd972dacead5deac692000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000670000000000000000000000000000000000000000000000000000000000000204069b0b8db5469b82390505d02cda4ca08bebf50ca0d824a88b6756fcd5aaf5e5000000000000000000000000c0bf8c1cdf9b6ca48656cb6b45c1c55df6d4997e000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000ce000000000000000000000000000000000000000000000000000000000000039107b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf0000000000000000000000009cedd5e1aa92f11df2eedffdbe9ab07190d652dd000000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000001d5000000000000000000000000000000000000000000000000000000000000037007b14d2b859efbbc97d68ca769d6e9b6a1c1fd9629d213e8555335448dc759bf000000000000000000000000d72dc5a49b9a7efc4ae5d18f963e46fddef22e2b0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000032a00000000000000000000000000000000000000000000000000000000000003e400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000a00000000000000000000000000000000000000000000000000000000000000030989b6d980d65c9d123f5358fd80de4aede8e32a8381b2be2227012489313ea1a251f0aa355bca68aa9caff52e7d74263000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000030b1b02cf60e45ae9e12df4d4d06c46cc6912343ea58465d44c290efd6c281cb4bcf137e86c908ffb6baf3282ce796632600000000000000000000000000000000";
        // assert that the built input bytes are the same as the expected bytes
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

    fn bytes_to_u32(bytes: &[u8]) -> Result<u32, &'static str> {
        // Ensure we have at least 4 bytes
        if bytes.len() < 4 {
            return Err("Input slice must be at least 4 bytes long");
        }

        // Take the last 4 bytes, regardless of the total length
        let last_four_bytes = &bytes[bytes.len() - 4..];

        // Convert to u32
        Ok(u32::from_be_bytes(last_four_bytes.try_into().unwrap()))
    }
}
