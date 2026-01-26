use anyhow::Result;
use std::sync::Arc;
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{
        contract::{CompiledClass, SierraClass},
        Call, Felt, BlockId, BlockTag, ExecutionResult, TransactionReceipt,
    },
    core::utils::get_selector_from_name,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};
use std::fs;

const RPC_URL: &str = "http://localhost:9944";
// UDC contract address from madara_addresses.json
const UDC_ADDRESS: &str = "0x50b90f8e476202b41e5d151b3691127b39acfc2a43ddcd6460bf725a2a1c659";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let private_key_hex = std::env::args()
        .nth(1)
        .expect("Usage: deploy_contract <private_key> <address> <contract_name> <constructor_args...>");
    let address_hex = std::env::args()
        .nth(2)
        .expect("Usage: deploy_contract <private_key> <address> <contract_name> <constructor_args...>");
    let contract_name = std::env::args()
        .nth(3)
        .expect("Usage: deploy_contract <private_key> <address> <contract_name> <constructor_args...>");

    // Collect constructor args
    let constructor_args: Vec<String> = std::env::args().skip(4).collect();

    println!("\n🚀 Deploying contract: {}", contract_name);
    println!("Account: {}", address_hex);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL)?));
    let chain_id = provider.chain_id().await?;

    // Setup account
    let private_key = SigningKey::from_secret_scalar(Felt::from_hex(&private_key_hex)?);
    let signer = LocalWallet::from(private_key);
    let address = Felt::from_hex(&address_hex)?;

    let mut account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        address,
        chain_id,
        starknet::accounts::ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

    // Load contract artifacts from Kamehameha
    let kamehameha_path = "/Users/heemankverma/Work/Karnot/RvsC/Kamehameha/compiled";
    let sierra_path = format!("{}/{}.sierra.json", kamehameha_path, contract_name);
    let casm_path = format!("{}/{}.casm.json", kamehameha_path, contract_name);

    println!("\n📄 Loading contract artifacts...");
    println!("Sierra: {}", sierra_path);
    println!("CASM: {}", casm_path);

    // Load and parse Sierra class
    let sierra_json = fs::read_to_string(&sierra_path)?;
    let sierra_class: SierraClass = serde_json::from_str(&sierra_json)?;

    // Get class hash before moving sierra_class
    let class_hash = sierra_class.class_hash()?;
    let flattened = sierra_class.flatten()?;

    // Load CASM for compiled class hash
    let casm_json = fs::read_to_string(&casm_path)?;
    let casm_class: CompiledClass = serde_json::from_str(&casm_json)?;
    let compiled_class_hash = casm_class.class_hash()?;

    println!("\n📝 Declaring contract...");
    let nonce = account.get_nonce().await?;
    println!("Current nonce: {:#066x}", nonce);

    // Check if already declared
    let already_declared = provider.get_class(BlockId::Tag(BlockTag::Latest), class_hash).await.is_ok();

    if already_declared {
        println!("⚠️  Class already declared, skipping declaration");
        println!("Class hash: {:#066x}", class_hash);
    } else {
        let declare_result = account
            .declare_v3(Arc::new(flattened), compiled_class_hash)
            .l1_gas(2000000000)
            .l2_gas(2000000000)
            .l1_data_gas(2000000)
            .send()
            .await?;

        println!("Declare transaction hash: {:#066x}", declare_result.transaction_hash);
        println!("Class hash: {:#066x}", declare_result.class_hash);

        // Wait for declare transaction
        println!("\n⏳ Waiting for declaration confirmation...");
        loop {
            match provider.get_transaction_receipt(declare_result.transaction_hash).await {
            Ok(receipt_info) => {
                match &receipt_info.receipt {
                    TransactionReceipt::Declare(receipt) => {
                        match &receipt.execution_result {
                            ExecutionResult::Succeeded => {
                                println!("✅ Declaration confirmed!");
                                break;
                            }
                            ExecutionResult::Reverted { reason } => {
                                println!("❌ Declaration reverted: {}", reason);
                                return Err(anyhow::anyhow!("Declaration failed"));
                            }
                        }
                    }
                    _ => {
                        println!("❌ Unexpected receipt type");
                        return Err(anyhow::anyhow!("Unexpected receipt type"));
                    }
                }
            }
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
        }
    }

    // Parse constructor calldata
    let mut constructor_calldata: Vec<Felt> = Vec::new();
    for arg in &constructor_args {
        constructor_calldata.push(Felt::from_hex(arg)?);
    }

    println!("\n📝 Deploying contract instance via UDC...");
    println!("Constructor calldata: {} args", constructor_calldata.len());

    // Build UDC call: deployContract(classHash, salt, unique, calldata)
    let udc_address = Felt::from_hex(UDC_ADDRESS)?;
    let salt = Felt::ZERO;
    let unique = Felt::ZERO; // false

    // Build calldata: [classHash, salt, unique, calldata_len, ...calldata]
    let mut udc_calldata = vec![
        class_hash,
        salt,
        unique,
        Felt::from(constructor_calldata.len()),
    ];
    udc_calldata.extend(constructor_calldata);

    let calls = vec![Call {
        to: udc_address,
        selector: get_selector_from_name("deployContract")?,
        calldata: udc_calldata,
    }];

    let deploy_tx = account
        .execute_v3(calls)
        .l1_gas(2000000000)
        .l2_gas(2000000000)
        .l1_data_gas(2000000)
        .send()
        .await?;

    println!("Deploy transaction hash: {:#066x}", deploy_tx.transaction_hash);

    // Wait for deploy transaction
    println!("\n⏳ Waiting for deployment confirmation...");
    let mut deployed_address = Felt::ZERO;
    loop {
        match provider.get_transaction_receipt(deploy_tx.transaction_hash).await {
            Ok(receipt_info) => {
                match &receipt_info.receipt {
                    TransactionReceipt::Invoke(receipt) => {
                        match &receipt.execution_result {
                            ExecutionResult::Succeeded => {
                                // Extract deployed address from ContractDeployed event
                                let contract_deployed_selector = get_selector_from_name("ContractDeployed")?;
                                for event in &receipt.events {
                                    if !event.keys.is_empty() && event.keys[0] == contract_deployed_selector {
                                        if !event.data.is_empty() {
                                            deployed_address = event.data[0];
                                        }
                                    }
                                }
                                println!("✅ Deployment confirmed!");
                                break;
                            }
                            ExecutionResult::Reverted { reason } => {
                                println!("❌ Deployment reverted: {}", reason);
                                return Err(anyhow::anyhow!("Deployment failed"));
                            }
                        }
                    }
                    _ => {
                        println!("❌ Unexpected receipt type");
                        return Err(anyhow::anyhow!("Unexpected receipt type"));
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }

    println!("\n✅ Successfully deployed {}!", contract_name);
    println!("Class hash: {:#066x}", class_hash);
    println!("Contract address: {:#066x}", deployed_address);

    Ok(())
}
