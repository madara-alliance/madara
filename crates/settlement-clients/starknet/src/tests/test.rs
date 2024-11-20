use std::env;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::eyre;
use rstest::{fixture, rstest};
use settlement_client_interface::SettlementClient;
use starknet::accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::contract::{CompiledClass, SierraClass};
use starknet::core::types::{
    BlockId, BlockTag, DeclareTransactionResult, Felt, FunctionCall, InvokeTransactionResult, StarknetError,
    TransactionExecutionStatus, TransactionStatus,
};
use starknet::macros::{felt, selector};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider, ProviderError, Url};
use starknet::signers::{LocalWallet, SigningKey};
use utils::env_utils::get_env_var_or_panic;

use super::setup::{wait_for_cond, MadaraCmd, MadaraCmdBuilder};
use crate::{LocalWalletSignerMiddleware, StarknetSettlementClient, StarknetSettlementValidatedArgs};

#[fixture]
pub async fn spin_up_madara() -> MadaraCmd {
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");
    tracing::debug!("Spinning up Madara");
    let mut node = MadaraCmdBuilder::new()
        .args([
            "--no-sync-polling",
            "--devnet",
            "--no-l1-sync",
            "--chain-config-path=./src/tests/preset.yml",
            "--rpc-cors",
            "all",
        ])
        .run();
    node.wait_for_ready().await;
    node
}

async fn wait_for_tx(account: &LocalWalletSignerMiddleware, transaction_hash: Felt, duration: Duration) -> bool {
    let result = wait_for_cond(
        || async {
            let receipt = match account.provider().get_transaction_status(transaction_hash).await {
                Ok(receipt) => Ok(receipt),
                Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound)) => {
                    Err(eyre!("Transaction not yet received"))
                }
                _ => Err(eyre!("Unknown error")),
            };

            match receipt {
                Ok(TransactionStatus::Received) => Err(eyre!("Transaction not yet received")),
                Ok(TransactionStatus::Rejected) => Ok(false),
                Ok(TransactionStatus::AcceptedOnL2(status)) => match status {
                    TransactionExecutionStatus::Succeeded => Ok(true),
                    TransactionExecutionStatus::Reverted => Ok(false),
                },
                Ok(TransactionStatus::AcceptedOnL1(status)) => match status {
                    TransactionExecutionStatus::Succeeded => Ok(true),
                    TransactionExecutionStatus::Reverted => Ok(false),
                },
                Err(e) => Err(eyre!("Unknown error: {}", e)),
            }
        },
        duration,
    )
    .await;
    match result {
        Ok(true) => true,
        Ok(false) => false,
        Err(e) => panic!("Error while waiting for transaction: {}", e),
    }
}

#[fixture]
async fn setup(#[future] spin_up_madara: MadaraCmd) -> (LocalWalletSignerMiddleware, MadaraCmd) {
    let madara_process = spin_up_madara.await;

    let starknet_settlement_params: StarknetSettlementValidatedArgs = StarknetSettlementValidatedArgs {
        starknet_rpc_url: Url::parse(madara_process.rpc_url.as_ref()).unwrap(),
        starknet_private_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_STARKNET_PRIVATE_KEY"),
        starknet_account_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_STARKNET_ACCOUNT_ADDRESS"),
        starknet_cairo_core_contract_address: get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_STARKNET_CAIRO_CORE_CONTRACT_ADDRESS",
        ),
        starknet_finality_retry_wait_in_secs: get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_STARKNET_FINALITY_RETRY_WAIT_IN_SECS",
        )
        .parse::<u64>()
        .unwrap(),
    };

    let rpc_url = Url::parse(starknet_settlement_params.starknet_rpc_url.as_ref()).unwrap();

    let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url)));
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(
        Felt::from_hex(&starknet_settlement_params.starknet_private_key).expect("Invalid private key"),
    ));
    let address = Felt::from_hex(&starknet_settlement_params.starknet_account_address.to_string()).unwrap();

    let chain_id = provider.chain_id().await.unwrap();
    let mut account = SingleOwnerAccount::new(provider, signer, address, chain_id, ExecutionEncoding::New);

    // `SingleOwnerAccount` defaults to checking nonce and estimating fees against the latest
    // block. Optionally change the target block to pending with the following line:
    account.set_block_id(BlockId::Tag(BlockTag::Pending));
    (Arc::new(account), madara_process)
}

#[rstest]
#[tokio::test]
async fn test_settle(#[future] setup: (LocalWalletSignerMiddleware, MadaraCmd)) {
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");

    let (account, madara_process) = setup.await;

    let mut starknet_settlement_params: StarknetSettlementValidatedArgs = StarknetSettlementValidatedArgs {
        starknet_rpc_url: madara_process.rpc_url.clone(),
        starknet_private_key: get_env_var_or_panic("MADARA_ORCHESTRATOR_STARKNET_PRIVATE_KEY"),
        starknet_account_address: get_env_var_or_panic("MADARA_ORCHESTRATOR_STARKNET_ACCOUNT_ADDRESS"),
        starknet_cairo_core_contract_address: get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_STARKNET_CAIRO_CORE_CONTRACT_ADDRESS",
        ),
        starknet_finality_retry_wait_in_secs: get_env_var_or_panic(
            "MADARA_ORCHESTRATOR_STARKNET_FINALITY_RETRY_WAIT_IN_SECS",
        )
        .parse::<u64>()
        .unwrap(),
    };

    let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
    let contract_path = project_root.join("crates/settlement-clients/starknet/src/tests/mock_contracts/target/dev");
    let sierra_class: SierraClass = serde_json::from_reader(
        std::fs::File::open(contract_path.join("mock_contracts_Piltover.contract_class.json"))
            .expect("Could not open sierra class file"),
    )
    .expect("Failed to parse SierraClass");

    let compiled_class: CompiledClass = serde_json::from_reader(
        std::fs::File::open(contract_path.join("mock_contracts_Piltover.compiled_contract_class.json"))
            .expect("Could not open compiled class file"),
    )
    .expect("Failed to parse CompiledClass");

    let flattened_class = sierra_class.clone().flatten().unwrap();
    let compiled_class_hash = compiled_class.class_hash().unwrap();

    let DeclareTransactionResult { transaction_hash: declare_tx_hash, class_hash: _ } =
        account.declare_v2(Arc::new(flattened_class.clone()), compiled_class_hash).send().await.unwrap();
    tracing::debug!("declare tx hash {:?}", declare_tx_hash);

    let is_success = wait_for_tx(&account, declare_tx_hash, Duration::from_secs(2)).await;
    assert!(is_success, "Declare transaction failed");

    let contract_factory = ContractFactory::new(flattened_class.class_hash(), account.clone());
    let deploy_v1 = contract_factory.deploy_v1(vec![], felt!("1122"), false);
    let deployed_address = deploy_v1.deployed_address();

    // env::set_var("STARKNET_CAIRO_CORE_CONTRACT_ADDRESS", deployed_address.to_hex_string());
    starknet_settlement_params.starknet_cairo_core_contract_address = deployed_address.to_hex_string();

    let InvokeTransactionResult { transaction_hash: deploy_tx_hash } =
        deploy_v1.send().await.expect("Unable to deploy contract");

    let is_success = wait_for_tx(&account, deploy_tx_hash, Duration::from_secs(2)).await;
    assert!(is_success, "Deploy trasaction failed");

    let settlement_client = StarknetSettlementClient::new_with_args(&starknet_settlement_params).await;
    let onchain_data_hash = [1; 32];
    let mut program_output = Vec::with_capacity(32);
    program_output.fill(onchain_data_hash);
    let update_state_tx_hash = settlement_client
        .update_state_calldata(program_output, onchain_data_hash, [1; 32])
        .await
        .expect("Sending Update state");

    tracing::debug!("update state tx hash {:?}", update_state_tx_hash);

    let is_success = wait_for_tx(
        &account,
        Felt::from_hex(&update_state_tx_hash).expect("Incorrect transaction hash"),
        Duration::from_secs(2),
    )
    .await;
    assert!(is_success, "Update state transaction failed/reverted");

    let call_result = account
        .provider()
        .call(
            FunctionCall {
                contract_address: deployed_address,
                entry_point_selector: selector!("get_is_updated"),
                calldata: vec![Felt::from_bytes_be_slice(&onchain_data_hash)],
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await
        .expect("failed to call the contract");
    assert!(call_result[0] == true.into(), "Should be updated");
}

#[rstest]
#[tokio::test]
async fn test_get_nonce_works(#[future] setup: (LocalWalletSignerMiddleware, MadaraCmd)) {
    let (account, _madara_process) = setup.await;
    let nonce = account.get_nonce().await;
    match &nonce {
        Ok(n) => tracing::debug!("Nonce value from get_nonce: {:?}", n),
        Err(e) => tracing::error!("Error getting nonce: {:?}", e),
    }
    assert!(nonce.is_ok(), "Failed to get nonce");
}
