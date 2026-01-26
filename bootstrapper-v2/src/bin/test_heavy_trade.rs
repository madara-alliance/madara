use starknet::{
    accounts::{Account, ConnectedAccount, ExecutionEncoding, SingleOwnerAccount},
    core::types::{Call, Felt},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
    signers::{LocalWallet, SigningKey},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse("http://localhost:9944").unwrap(),
    ));

    let signer = LocalWallet::from(SigningKey::from_secret_scalar(
        Felt::from_hex("0x063bb0bb54374247449a9f6f978a3653b969e2a1294b35205d721ebc1539446a").unwrap(),
    ));

    let account_address = Felt::from_hex("0x012aa6059457fc2d02240962a6573e051fa919632853e6ba70207d7cef6be4c3").unwrap();
    let chain_id = Felt::from_hex("0x4d4144415241").unwrap();

    let account = SingleOwnerAccount::new(
        provider,
        signer,
        account_address,
        chain_id,
        ExecutionEncoding::New,
    );

    let contract_address = Felt::from_hex("0x04c649f70e5598cddfed10e115cf64599d1e14f49ea6b2c028d40fcb9a548520").unwrap();
    let market_id = Felt::from(1);
    let maker = account_address;
    let taker = Felt::from_hex("0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b").unwrap();

    // Selectors from ABI
    let init_market_selector = Felt::from_hex("0x0c01b0105f602e73419fde7598d5c06620bb9ac5913157c4c218f0452dbb24e").unwrap();
    let init_account_selector = Felt::from_hex("0x032a8cb1cf5de02dd990b015d003b80694ca328b0765dcce21a457b69c40fbb7").unwrap();
    let trade_selector = Felt::from_hex("0x0395d3e003b1dc991410c4903dde3cc31ab66717268812745463ef50e51349dc").unwrap();

    println!("🔧 Initializing HeavyTradeSimulator...");
    println!("Contract: {:#064x}\n", contract_address);

    // Step 1: Initialize market
    println!("📝 Step 1: Initializing market {}...", market_id);
    let init_market_call = Call {
        to: contract_address,
        selector: init_market_selector,
        calldata: vec![market_id],
    };

    // Step 2: Initialize maker account
    println!("📝 Step 2: Initializing maker account...");
    let init_maker_call = Call {
        to: contract_address,
        selector: init_account_selector,
        calldata: vec![
            maker,
            market_id,
            Felt::from(10000000000u128), // 10B balance
        ],
    };

    // Step 3: Initialize taker account
    println!("📝 Step 3: Initializing taker account...");
    let init_taker_call = Call {
        to: contract_address,
        selector: init_account_selector,
        calldata: vec![
            taker,
            market_id,
            Felt::from(10000000000u128), // 10B balance
        ],
    };

    // Execute all initializations
    let init_result = account.execute_v3(vec![init_market_call, init_maker_call, init_taker_call])
        .l1_gas(100000000)
        .l2_gas(100000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Initialization TX: {:#064x}", init_result.transaction_hash);
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let init_receipt = account.provider().get_transaction_receipt(init_result.transaction_hash).await?;
    println!("✅ Initialization result: {:?}\n", init_receipt.receipt.execution_result());

    // Step 4: Execute heavy trade simulation
    println!("💰 Step 4: Executing heavy trade simulation...");
    println!("Maker: {:#064x}", maker);
    println!("Taker: {:#064x}", taker);
    println!("Market: {}", market_id);
    println!("Size: 1,000,000 units");
    println!("Price: 50,000\n");

    let trade_call = Call {
        to: contract_address,
        selector: trade_selector,
        calldata: vec![
            maker,
            taker,
            market_id,
            Felt::from(1000000u128),  // trade_size
            Felt::from(50000u128),    // trade_price
            Felt::ZERO,               // is_spot (false = perpetual)
        ],
    };

    let trade_result = account.execute_v3(vec![trade_call])
        .l1_gas(100000000)
        .l2_gas(100000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Trade TX: {:#064x}", trade_result.transaction_hash);
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let trade_receipt = account.provider().get_transaction_receipt(trade_result.transaction_hash).await?;
    println!("\n✅ Trade simulation complete!");
    println!("Execution result: {:?}", trade_receipt.receipt.execution_result());
    println!("Events count: {}", trade_receipt.receipt.events().len());

    Ok(())
}
