//! End-to-end test for the local-aggregation path used by the Mock prover.
//!
//! Flow (all in-process):
//! 1. Download four SNOS `CairoPIE` zips (Starknet Sepolia blocks 8059997..=8060000)
//!    from the `madara-test-artifacts` repo.
//! 2. Extract each PIE's program output.
//! 3. Run the local aggregator ([`orchestrator_aggregator_runner::run_local_aggregator`])
//!    against those outputs.
//! 4. Compute the aggregator fact hash via [`get_fact_info`] and assert it matches the
//!    pinned expected value (catches drift in aggregator / fact computation).
//! 5. Spawn anvil, deploy `MockGpsVerifier` (bytecode inlined from Foundry build).
//! 6. Register the fact via [`FactRegistrar::ensure_registered`]; assert the tx hash is
//!    non-zero and that a second call short-circuits to `B256::ZERO` (idempotency).
//! 7. Cross-check [`FactRegistrar::is_registered`] returns `true`.
//!
//! Requires `MADARA_ORCHESTRATOR_TEST_DATA_BASE_URL` (set by `.env.test` / CI) and the
//! Python Cairo venv (set up in CI for the aggregator-runner's `starknet_os` dep).

use std::str::FromStr;

use alloy::network::EthereumWallet;
use alloy::node_bindings::Anvil;
use alloy::primitives::B256;
use alloy::providers::ProviderBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use orchestrator_aggregator_runner::{run_local_aggregator, AggregatorFelt, AggregatorRunnerInput, PROGRAM_HASHES};
use orchestrator_gps_fact_checker::FactRegistrar;
use orchestrator_utils::test_utils::setup_test_data;
use rstest::rstest;

use crate::worker::utils::fact_info::{get_fact_info, get_program_output};

// MockGpsVerifier bytecode from `orchestrator/contracts/mock-verifier/`.
//
// Regenerate by running `forge build` in that directory and copying
// `.bytecode.object` from `out/MockGpsVerifier.sol/MockGpsVerifier.json`.
sol! {
    #[allow(missing_docs)]
    #[sol(rpc, bytecode = "0x608060405234801561000f575f80fd5b5061011b8061001d5f395ff3fe6080604052348015600e575f80fd5b50600436106030575f3560e01c80636a9385671460345780636d5d6eb7146067575b5f80fd5b6053603f36600460cf565b5f9081526020819052604090205460ff1690565b604051901515815260200160405180910390f35b6076607236600460cf565b6078565b005b5f8181526020819052604090205460ff1660cc575f81815260208190526040808220805460ff191660011790555182917f9b55a62205cf41784a81d1eb40ac16f5c1347d324e28065a742c53564b9c6a3291a25b50565b5f6020828403121560de575f80fd5b503591905056fea26469706673582212200f5923794f33b5c2d4a2101a0b121858788f645241e44cc29046fd045beb066064736f6c63430008180033")]
    contract MockGpsVerifier {
        event FactRegistered(bytes32 indexed fact);
        function registerFact(bytes32 fact) external;
        function isValid(bytes32 fact) external view returns (bool);
    }
}

/// Chain id the SNOS PIEs were produced on (ASCII `"SN_SEPOLIA"`).
const SEPOLIA_CHAIN_ID_HEX: &str = "0x534e5f5345504f4c4941";
/// STRK fee token address returned by Sepolia feeder gateway `get_contract_addresses`.
const SEPOLIA_STRK_FEE_TOKEN: &str = "0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

/// Filenames in the `madara-test-artifacts` repo. Each zip is a SNOS CairoPIE for one
/// Starknet Sepolia block.
const SNOS_PIE_FILES: [&str; 4] =
    ["snos_sepolia_8059997.zip", "snos_sepolia_8059998.zip", "snos_sepolia_8059999.zip", "snos_sepolia_8060000.zip"];

/// Expected aggregator fact hash when the four SNOS PIEs above are aggregated.
/// Pinning this value turns any drift in `run_local_aggregator` or `get_fact_info` into
/// a loud test failure.
/// State update transaction with this fact - https://sepolia.etherscan.io/tx/0x210ce8de99bfc31ddf1d6c62665210bb23c9ec975fd1a6ff7ce0c7124c8e6140
const EXPECTED_AGG_FACT: &str = "0x567583b3f11c269a52e29622b95e7be9b0a1a195c0df9ad3e052b4c82163a6dc";

#[rstest]
#[tokio::test(flavor = "multi_thread")]
async fn test_local_aggregator_with_fact_registration() -> color_eyre::Result<()> {
    dotenvy::from_filename_override("../.env.test").ok();

    // 1. Pull the SNOS PIEs from the remote artifacts repo.
    let data_dir = setup_test_data(SNOS_PIE_FILES.iter().map(|f| (*f, false)).collect())
        .await
        .expect("Failed to download SNOS PIE artifacts");

    // 2. Extract program outputs (is_aggregator = false — these are SNOS PIEs).
    let mut child_program_outputs: Vec<Vec<[u8; 32]>> = Vec::with_capacity(SNOS_PIE_FILES.len());
    for name in SNOS_PIE_FILES {
        let pie = CairoPie::read_zip_file(&data_dir.path().join(name))
            .unwrap_or_else(|e| panic!("Failed to read SNOS PIE {name}: {e}"));
        let output = get_program_output(&pie, false)
            .unwrap_or_else(|e| panic!("Failed to extract program output from {name}: {e}"));
        child_program_outputs.push(output.iter().map(|f| f.to_bytes_be()).collect());
    }

    // 3. Run local aggregator.
    let input = AggregatorRunnerInput {
        child_program_outputs,
        layout: LayoutName::all_cairo,
        full_output: false,
        debug_mode: false,
        chain_id: AggregatorFelt::from_hex(SEPOLIA_CHAIN_ID_HEX).expect("valid Sepolia chain id"),
        fee_token_address: AggregatorFelt::from_hex(SEPOLIA_STRK_FEE_TOKEN).expect("valid Sepolia fee token address"),
        da_public_keys: None,
    };
    let agg_output = run_local_aggregator(input).expect("Local aggregator failed");

    // 4. Compute the aggregator fact hash. SHARP's on-chain verifier uses the
    //    applicative-bootloader-wrapped aggregator program hash (`aggregator_with_prefix`),
    //    not the bare aggregator program embedded in the PIE.
    let agg_pie = &agg_output.aggregator_cairo_pie;
    let fact_info = get_fact_info(agg_pie, Some(PROGRAM_HASHES.aggregator_with_prefix), true)?;
    let fact: [u8; 32] = fact_info.fact.0;
    let fact_hex = format!("0x{}", hex::encode(fact));
    assert_eq!(fact_hex, EXPECTED_AGG_FACT, "aggregator fact hash drifted");

    // 5. Spawn anvil and deploy MockGpsVerifier using the inlined bytecode.
    let anvil = Anvil::new().try_spawn().expect("Failed to spawn anvil");
    let signer = PrivateKeySigner::from_str(&hex::encode(anvil.keys()[0].to_bytes()))
        .expect("Failed to build signer from anvil dev key");
    let wallet = EthereumWallet::from(signer.clone());
    let deploy_provider = ProviderBuilder::new().wallet(wallet).connect_http(anvil.endpoint_url());

    let verifier = MockGpsVerifier::deploy(deploy_provider).await.expect("Failed to deploy MockGpsVerifier");
    let verifier_address = *verifier.address();

    // 6. Register the fact via FactRegistrar.
    let registrar = FactRegistrar::new(anvil.endpoint_url(), signer, verifier_address);

    // Sanity: isValid before registration must be false.
    assert!(!registrar.is_registered(fact).await?, "fact unexpectedly already registered pre-tx");

    let tx_hash = registrar.ensure_registered(fact).await.expect("ensure_registered failed");
    assert_ne!(tx_hash, B256::ZERO, "first ensure_registered should send a tx and return a non-zero hash");

    // 7. Cross-check on-chain state + idempotency short-circuit.
    assert!(registrar.is_registered(fact).await?, "isValid returned false despite tx success");
    let tx_hash2 = registrar.ensure_registered(fact).await.expect("second ensure_registered failed");
    assert_eq!(tx_hash2, B256::ZERO, "second ensure_registered should short-circuit (fact already registered)");

    Ok(())
}
