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

    let contract_address = Felt::from_hex("0x00cf5d9d10b410e8732ff040c3dc32c2d2e70314b5eb7bc14f9df8c727587e15").unwrap();
    let market_id = Felt::from(1);
    let maker = account_address;
    let taker = Felt::from_hex("0x0492a34ada7acb0dda8636b9d36a22cd4f6bb50b6421f1bd64d098de60835f9b").unwrap();

    println!("🔧 Initializing HeavyTradeSimulator...");
    println!("Contract: {:#064x}\n", contract_address);

    // Get selectors from chain
    let init_market_selector = Felt::from_hex("0x030ff0de1b8b3a91c4c4c7c7e7d6a5f0e8d24d1c45d3e9c9d5f5f5f5f5f5f5f5").unwrap(); // placeholder
    let init_account_selector = Felt::from_hex("0x030ff0de1b8b3a91c4c4c7c7e7d6a5f0e8d24d1c45d3e9c9d5f5f5f5f5f5f5f5").unwrap(); // placeholder
    let trade_selector = Felt::from_hex("0x0395d3e003b1dc991410c4903dde3cc31ab66717268812745463ef50e51349dc").unwrap();

    // Calculate selectors properly
    let init_market_selector = {
        use std::process::Command;
        let output = Command::new("python3")
            .arg("-c")
            .arg("import hashlib; name = 'initialize_market'; hash_obj = hashlib.new('sha3_256'); hash_obj.update(name.encode('ascii')); h = hash_obj.hexdigest(); selector = int(h, 16) & ((1 << 251) - 1); print(hex(selector))")
            .output()?;
        Felt::from_hex(std::str::from_utf8(&output.stdout)?.trim())?
    };

    let init_account_selector = {
        use std::process::Command;
        let output = Command::new("python3")
            .arg("-c")
            .arg("import hashlib; name = 'initialize_account'; hash_obj = hashlib.new('sha3_256'); hash_obj.update(name.encode('ascii')); h = hash_obj.hexdigest(); selector = int(h, 16) & ((1 << 251) - 1); print(hex(selector))")
            .output()?;
        Felt::from_hex(std::str::from_utf8(&output.stdout)?.trim())?
    };

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

    // Execute all initializations in one transaction
    let init_result = account.execute_v3(vec![init_market_call, init_maker_call, init_taker_call])
        .l1_gas(100000000)
        .l2_gas(100000000)
        .l1_data_gas(100000)
        .send()
        .await?;

    println!("Initialization TX: {:#064x}", init_result.transaction_hash);
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    let init_receipt = account.provider().get_transaction_receipt(init_result.transaction_hash).await?;
    println!("✅ Initialization complete!\n");

    // Step 4: Execute heavy trade simulation
    println!("💰 Step 4: Executing heavy trade simulation...");
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
    println!("✅ Trade simulation complete!\n");
    println!("Receipt: {:?}", trade_receipt);

    Ok(())
}
