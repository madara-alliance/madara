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
    let chain_id = Felt::from_hex("0x4d4144415241").unwrap(); // MADARA

    let account = SingleOwnerAccount::new(
        provider,
        signer,
        account_address,
        chain_id,
        ExecutionEncoding::New,
    );

    println!("💰 Calling HeavyTradeSimulator heavy_trade_simulation()...");
    println!("From account: {:#064x}", account_address);
    println!("Contract: 0x0689386ac257e6bc104126a40c5309f2d52304cfb51a5b1699684c7bead60afb\n");

    let contract_address = Felt::from_hex("0x0689386ac257e6bc104126a40c5309f2d52304cfb51a5b1699684c7bead60afb").unwrap();

    // Selector for heavy_trade_simulation (function_idx 0)
    let selector = Felt::from_hex("0x0395d3e003b1dc991410c4903dde3cc31ab66717268812745463ef50e51349dc").unwrap();

    // Prepare calldata for heavy_trade_simulation:
    // fn heavy_trade_simulation(maker, taker, market_id, trade_size, trade_price, is_spot)
    let maker = account_address;
    let taker = Felt::from_hex("0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b").unwrap(); // bootstrap account
    let market_id = Felt::from(1); // Market ID 1
    let trade_size = Felt::from(1000000u128); // 1M units
    let trade_price = Felt::from(50000u128); // Price: 50000
    let is_spot = Felt::ZERO; // false (perpetual trade)

    let calldata = vec![
        maker,
        taker,
        market_id,
        trade_size,     // u128 fits in felt252
        trade_price,    // u128 fits in felt252
        is_spot,        // bool
    ];

    let call = Call {
        to: contract_address,
        selector,
        calldata,
    };

    println!("📝 Executing heavy trade simulation...");
    println!("Maker: {:#064x}", maker);
    println!("Taker: {:#064x}", taker);
    println!("Market ID: {}", market_id);
    println!("Trade Size: {} units", trade_size);
    println!("Trade Price: {}", trade_price);
    println!("Type: Perpetual\n");

    let result = account.execute_v3(vec![call])
        .l1_gas(100000000)
        .l2_gas(100000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Transaction hash: {:#064x}\n", result.transaction_hash);
    println!("⏳ Waiting for transaction confirmation...");

    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let receipt = account.provider().get_transaction_receipt(result.transaction_hash).await?;
    println!("✅ Transaction confirmed!\n");
    println!("Receipt: {:?}", receipt);

    Ok(())
}
