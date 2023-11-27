//! Utility functions.
use reqwest;
use rand::seq::SliceRandom;
use rand::thread_rng;
use serde_json::Value;
use tokio;

/// Returns the block number of the last block synced by Substrate.
pub async fn get_last_synced_block(rpc_port: u16) -> u64 {
    let client = reqwest::Client::new();

    let url = format!("http://localhost:{}/", rpc_port);
    let payload = serde_json::to_vec(&serde_json::json!({
        "id": 1,
        "jsonrpc": "2.0",
        "method": "chain_getBlock",
        "params": []
    }))
    .unwrap();

    let response: serde_json::Value = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body(payload)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let number_as_hex = response["result"]["block"]["header"]["number"].as_str().unwrap();
    u64::from_str_radix(&number_as_hex[2..], 16).unwrap()
}

/// Returns a random Pokémon name.
pub async fn get_random_pokemon_name() -> Result<String, Box<dyn std::error::Error>> {
    let res = reqwest::get("https://pokeapi.co/api/v2/pokemon/?limit=1000").await?;
    let body = res.text().await?;
    let json: Value = serde_json::from_str(&body)?;

    let pokemon_array = json["results"].as_array().unwrap();
    let mut rng = thread_rng();
    let random_pokemon = pokemon_array.choose(&mut rng).unwrap();

    Ok(random_pokemon["name"].as_str().unwrap().to_string())
}