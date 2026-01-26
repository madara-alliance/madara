use starknet::{
    core::types::{BlockId, BlockTag, Call, Felt, FunctionCall},
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider, Url},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse("http://localhost:9944").unwrap(),
    ));

    let contract_address = Felt::from_hex("0x0689386ac257e6bc104126a40c5309f2d52304cfb51a5b1699684c7bead60afb").unwrap();

    // Selector for get_total_hashes_computed (function_idx 3)
    let selector = Felt::from_hex("0x015e4b5c7cd5a1ad03c1be5b94ece3484ff92ef7379c89b0da77dffe59f56be3").unwrap();

    println!("📊 Calling HeavyTradeSimulator get_total_hashes_computed()...");
    println!("Contract: {:#064x}\n", contract_address);

    let call = FunctionCall {
        contract_address,
        entry_point_selector: selector,
        calldata: vec![],
    };

    let result = provider.call(call, BlockId::Tag(BlockTag::Latest)).await?;

    println!("✅ Result: {:?}", result);
    println!("\nTotal hashes computed: {}", result.get(0).unwrap_or(&Felt::ZERO));

    Ok(())
}
