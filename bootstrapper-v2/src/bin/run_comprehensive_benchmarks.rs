use anyhow::Result;
use starknet::{
    accounts::{Account, ConnectedAccount, SingleOwnerAccount},
    core::types::{Call, Felt, BlockId, BlockTag, ExecutionResult, TransactionReceipt},
    core::utils::get_selector_from_name,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};
use std::fs::File;
use std::io::Write as IoWrite;

const RPC_URL: &str = "http://localhost:9944";

// ComprehensiveBenchmark contract details (V4 - with i128 changes)
const CONTRACT_ADDRESS: &str = "0x025cf1785fa9dd2b1ef0b5d2fc5df8526897bbe1244d3e20a0f984bef1cb1985";
const ACCOUNT_ADDRESS: &str = "0x012aa6059457fc2d02240962a6573e051fa919632853e6ba70207d7cef6be4c3";
const ACCOUNT_PRIVATE_KEY: &str = "0x063bb0bb54374247449a9f6f978a3653b969e2a1294b35205d721ebc1539446a";

// All 28 benchmark functions
const BENCHMARK_FUNCTIONS: &[&str] = &[
    // A. Storage Benchmarks (6)
    "benchmark_storage_read",
    "benchmark_storage_write",
    "benchmark_map_read",
    "benchmark_map_write",
    "benchmark_nested_map_read",
    "benchmark_nested_map_write",

    // B. Hashing Benchmarks (3)
    "benchmark_pedersen_hashing",
    "benchmark_pedersen_parallel",
    "benchmark_poseidon_hashing",

    // C. Math Benchmarks (8)
    "benchmark_math_division",
    "benchmark_math_multiplication",
    "benchmark_math_addition",
    "benchmark_math_subtraction",
    "benchmark_math_signed_division",
    "benchmark_math_signed_multiplication",
    "benchmark_math_combined_ops",
    "benchmark_math_signed_combined",

    // D. Control Flow Benchmarks (3)
    "benchmark_loops",
    "benchmark_conditionals",
    "benchmark_assertions",

    // E. Complex Pattern Benchmarks (5)
    "benchmark_struct_read",
    "benchmark_struct_write",
    "benchmark_multi_map_coordination",
    "benchmark_referential_reads",
    "benchmark_batch_operations",

    // F. End-to-End Benchmarks (3)
    "benchmark_simple_transaction",
    "benchmark_medium_transaction",
    "benchmark_heavy_transaction",
];

// Iteration values to test
const ITERATIONS: &[u128] = &[1, 10, 100, 500, 1000];

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    println!("\n🚀 Starting Comprehensive Benchmark Suite");
    println!("═══════════════════════════════════════════════════════════");
    println!("Contract: {}", CONTRACT_ADDRESS);
    println!("Functions: {}", BENCHMARK_FUNCTIONS.len());
    println!("Iterations per function: {}", ITERATIONS.len());
    println!("Total transactions: {}", BENCHMARK_FUNCTIONS.len() * ITERATIONS.len());
    println!("═══════════════════════════════════════════════════════════\n");

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(RPC_URL)?));
    let chain_id = provider.chain_id().await?;

    // Setup account
    let private_key = SigningKey::from_secret_scalar(Felt::from_hex(ACCOUNT_PRIVATE_KEY)?);
    let signer = LocalWallet::from(private_key);
    let address = Felt::from_hex(ACCOUNT_ADDRESS)?;

    let mut account = SingleOwnerAccount::new(
        provider.clone(),
        signer,
        address,
        chain_id,
        starknet::accounts::ExecutionEncoding::New,
    );
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

    let contract_address = Felt::from_hex(CONTRACT_ADDRESS)?;

    // Get initial nonce and track it manually
    let mut current_nonce = account.get_nonce().await?;
    println!("Initial nonce: {:#066x}\n", current_nonce);

    // Storage for results: [function_name][iteration_index] = tx_hash
    let mut results: Vec<Vec<String>> = vec![vec![String::new(); ITERATIONS.len()]; BENCHMARK_FUNCTIONS.len()];

    // Execute all benchmarks
    let mut total_executed = 0;
    let total_benchmarks = BENCHMARK_FUNCTIONS.len() * ITERATIONS.len();

    for (fn_idx, function_name) in BENCHMARK_FUNCTIONS.iter().enumerate() {
        println!("\n📊 Testing function: {}", function_name);
        println!("─────────────────────────────────────────────────────────");

        for (iter_idx, &iteration_count) in ITERATIONS.iter().enumerate() {
            total_executed += 1;

            print!("  [{}/{}] Calling with iterations={:>4}... ",
                   total_executed, total_benchmarks, iteration_count);
            std::io::stdout().flush()?;

            // Build the call
            let selector = get_selector_from_name(function_name)?;
            let calldata = vec![
                Felt::from(iteration_count), // iterations parameter (u128 is a single felt)
            ];

            let call = Call {
                to: contract_address,
                selector,
                calldata,
            };

            // Get current block number before sending transaction
            let block_before = match provider.block_number().await {
                Ok(num) => num,
                Err(e) => {
                    println!("❌ Failed to get block number: {}", e);
                    results[fn_idx][iter_idx] = format!("ERROR: Failed to get block number");
                    continue;
                }
            };

            // Execute the transaction with manually tracked nonce
            match account
                .execute_v3(vec![call])
                .nonce(current_nonce)
                .l1_gas(2000000000)
                .l2_gas(2000000000)
                .l1_data_gas(2000000)
                .send()
                .await
            {
                Ok(result) => {
                    let tx_hash = format!("{:#066x}", result.transaction_hash);

                    // Transaction was sent successfully, so nonce will be consumed even if it reverts
                    // Increment nonce immediately after send
                    current_nonce += Felt::ONE;

                    // Wait for confirmation
                    match wait_for_transaction(&provider, result.transaction_hash).await {
                        Ok(_) => {
                            println!("✅ {}", tx_hash);
                            results[fn_idx][iter_idx] = tx_hash;
                        }
                        Err(e) => {
                            println!("❌ Failed (reverted): {}", e);
                            results[fn_idx][iter_idx] = format!("REVERTED: {}", tx_hash);
                        }
                    }

                    // Wait for next block (block time is 2 seconds)
                    print!("      Waiting for new block... ");
                    std::io::stdout().flush()?;

                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        match provider.block_number().await {
                            Ok(current_block) => {
                                if current_block > block_before {
                                    println!("Block #{} → #{}", block_before, current_block);
                                    break;
                                }
                            }
                            Err(_) => {
                                // Continue waiting if we can't get block number
                                continue;
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("❌ Send failed: {}", e);
                    results[fn_idx][iter_idx] = format!("ERROR: {}", e);
                }
            }
        }
    }

    // Write results to CSV
    let csv_path = "/Users/heemankverma/Work/Karnot/RvsC/stats/txn_hash_to_fn_name_with_iteration_count.csv";
    println!("\n\n💾 Writing results to CSV: {}", csv_path);

    let mut csv_file = File::create(csv_path)?;

    // Write header row (iterations as columns)
    write!(csv_file, "Function")?;
    for iteration in ITERATIONS {
        write!(csv_file, ",iter_{}", iteration)?;
    }
    writeln!(csv_file)?;

    // Write data rows (one row per function)
    for (fn_idx, function_name) in BENCHMARK_FUNCTIONS.iter().enumerate() {
        write!(csv_file, "{}", function_name)?;
        for iter_idx in 0..ITERATIONS.len() {
            write!(csv_file, ",{}", results[fn_idx][iter_idx])?;
        }
        writeln!(csv_file)?;
    }

    println!("✅ CSV file written successfully!");
    println!("\n═══════════════════════════════════════════════════════════");
    println!("🎉 Benchmark suite completed!");
    println!("Total transactions executed: {}/{}", total_executed, total_benchmarks);
    println!("═══════════════════════════════════════════════════════════\n");

    Ok(())
}

async fn wait_for_transaction(
    provider: &JsonRpcClient<HttpTransport>,
    tx_hash: Felt,
) -> Result<()> {
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(receipt_info) => {
                match &receipt_info.receipt {
                    TransactionReceipt::Invoke(receipt) => {
                        match &receipt.execution_result {
                            ExecutionResult::Succeeded => {
                                return Ok(());
                            }
                            ExecutionResult::Reverted { reason } => {
                                return Err(anyhow::anyhow!("Transaction reverted: {}", reason));
                            }
                        }
                    }
                    _ => {
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
