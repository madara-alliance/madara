use anyhow::Result;
use starknet::{
    core::types::Felt,
    signers::{SigningKey, VerifyingKey},
};

#[tokio::main]
async fn main() -> Result<()> {
    let private_key_hex = std::env::args()
        .nth(1)
        .expect("Usage: get_public_key <private_key>");

    let private_key = SigningKey::from_secret_scalar(Felt::from_hex(&private_key_hex)?);
    let public_key = private_key.verifying_key();

    println!("Private key: {}", private_key_hex);
    println!("Public key: {:#066x}", public_key.scalar());

    Ok(())
}
