use std::env;
use std::path::PathBuf;
use std::str::FromStr;

use alloy::node_bindings::{Anvil, AnvilInstance};
use alloy::primitives::Address;
use alloy::providers::ext::AnvilApi;
use alloy::providers::ProviderBuilder;
use alloy::sol;
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
        dotenvy::from_filename(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

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

    use alloy::eips::eip4844::BYTES_PER_BLOB;
    use alloy::primitives::Address;
    use alloy::providers::Provider;
    use alloy::sol_types::private::U256;
    use alloy_primitives::FixedBytes;
    use color_eyre::eyre::eyre;
    use rstest::rstest;
    use settlement_client_interface::{SettlementClient, SettlementVerificationStatus};
    use tokio::time::sleep;
    use utils::env_utils::get_env_var_or_panic;

    use super::{BLOCK_TIME, ENV_FILE_PATH};
    use crate::conversion::to_padded_hex;
    use crate::tests::{
        DummyCoreContract, EthereumTestBuilder, Pipe, CURRENT_PATH, MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS,
        STARKNET_CORE_CONTRACT, STARKNET_CORE_CONTRACT_ADDRESS,
    };
    use crate::types::{bytes_be_to_u128, convert_stark_bigint_to_u256};
    use crate::{EthereumSettlementClient, EthereumSettlementValidatedArgs, Y_HIGH_POINT_OFFSET, Y_LOW_POINT_OFFSET};

    #[rstest]
    #[tokio::test]
    /// Tests if the method is able to do a transaction with same function selector on a dummy
    /// contract. If we impersonate starknet operator then we loose out on testing for validity
    /// of signature in the transaction. Starknet core contract has a modifier `onlyOperator`
    /// that restricts anyone but the operator to send transaction to `updateStateKzgDa` method
    /// And hence to test the signature and transaction via a dummy contract that has same function
    /// selector as `updateStateKzgDa`. and anvil is for testing on fork Eth.
    async fn update_state_blob_with_dummy_contract_works() {
        dotenvy::from_filename(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

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
        let program_output = vec![[0; 32]; 16];
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
        dotenvy::from_filename(&*ENV_FILE_PATH).expect("Could not load .env.test file.");

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
        dotenvy::from_filename(&*ENV_FILE_PATH).expect("Could not load .env.test file.");
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
        };

        let ethereum_settlement_client = EthereumSettlementClient::with_test_params(
            setup.provider.clone(),
            ethereum_settlement_params.l1_core_contract_address,
            ethereum_settlement_params.ethereum_rpc_url,
            None,
        );
        assert_eq!(
            ethereum_settlement_client.get_last_settled_block().await.expect("Could not get last settled block."),
            218378
        );
    }

    #[rstest]
    #[tokio::test]
    #[case::basic(6806848)]
    async fn creating_input_data_works(#[case] fork_block_no: u64) {
        use c_kzg::Bytes32;

        use crate::conversion::get_input_data_for_eip_4844;

        let program_output = get_program_output(fork_block_no);
        let blob_data_vec = get_blob_data(fork_block_no);

        let x_0_value_bytes32 = Bytes32::from(program_output[10]);
        let y_0 = Bytes32::from(
            convert_stark_bigint_to_u256(
                bytes_be_to_u128(&program_output[Y_LOW_POINT_OFFSET]),
                bytes_be_to_u128(&program_output[Y_HIGH_POINT_OFFSET]),
            )
            .to_be_bytes(),
        );

        let kzg_proof = EthereumSettlementClient::build_proof(blob_data_vec, x_0_value_bytes32, y_0)
            .expect("Unable to build KZG proof for given params.")
            .to_owned();

        let input_bytes = get_input_data_for_eip_4844(program_output, kzg_proof).expect("unable to create input data");
        let expected = "0x507ee52800000000000000000000000000000000000000000000000000000000000000400000000000000000000000000000000000000000000000000000000000000340000000000000000000000000000000000000000000000000000000000000001701159ac740283722faa4471b19bd217faf0cc51f1b999308b8fc88785934bab4065bf49c042ee1779db664138a3e7e749ac221e5ea7b645717afee0707671d1a000000000000000000000000000000000000000000000000000000000003550a000000000000000000000000000000000000000000000000000000000003552200dee83a5fc2a2bdc20e74a55394fe5d8701148b6d3e95057e03d54b5053370104f67725b439c46eaaae6af772cf254706f6c0ebe870d8b3bbd5e40a5226148e01e324682835e60c4779a683b32713504aed894fd73842f7d05b18e7bd29cd70000504fa6e5eb930c0d8329d4a77d98391f2730dab8516600aeaf733a6123432000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000000277d5eca17f14b34dc8ca9e87d0f88fb0a5b6f71a5580b19244e6426c63b453000000000000000000000000000000000000000000000000000000000000000100000000000000009c2e6b2dce7fc7bcce9d4351c65bb7221755f09ee6f37c00000000000000000092dd12a200752ae9e63b5985ce6ab169e41ce94dac0ed03e00000000000000000000000000000000bdde84d7e9b9947c940b47083d320d330000000000000000000000000000000016bcc30c68f19429e08cc07b5a63ce6f000000000000000000000000000000000000000000000000000000000000000500daf7c17ae6ecc0379a2a1a5a19d7fa2db03dd7ae98d15f2c5ae22b877c84c90000000000000000000000009bdbfca4fefb51f83e9c5dbcfa53661b3b3deba30000000000000000000000000000000000000000000000000000000000000002069e95c78dd84fe0706b39476b0454ff1f9ecbbce43b9456c1bbf18d54469af405b3e92711ef23d62955df1f6975e0db2a360c5f34ab2532bb1c89a48e25f2720000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000030b773cc46ff56ace65907afc646b5822808c0a01b223afa4f7bc8d9a8d73f3f398d3098fe69255956599d9c2d565b083400000000000000000000000000000000";
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
