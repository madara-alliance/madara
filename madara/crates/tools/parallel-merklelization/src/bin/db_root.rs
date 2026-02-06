use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use mc_db::rocksdb::{RocksDBConfig, RocksDBStorage};
use mc_db::storage::StorageChainTip;
use mc_db::{MadaraStorageRead, MadaraStorageWrite};
use mp_convert::Felt;
use reqwest::blocking::Client as BlockingClient;
use serde::de::DeserializeOwned;
use starknet_core::types::MaybePreConfirmedBlockWithTxHashes;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "db-root", about = "Read current state root from a Madara DB")]
struct Args {
    #[arg(long, value_name = "PATH")]
    db_path: PathBuf,
    #[arg(long, value_name = "URL")]
    rpc_url: Option<String>,
}

fn resolve_db_path(input: &Path) -> Result<PathBuf> {
    if input.join("CURRENT").exists() {
        return Ok(input.to_path_buf());
    }
    let nested = input.join("db");
    if nested.join("CURRENT").exists() {
        return Ok(nested);
    }
    bail!("DB path does not look like a RocksDB directory: {}", input.display())
}

fn format_felt(value: Felt) -> String {
    format!("0x{:x}", value)
}

fn block_id(block_n: u64) -> serde_json::Value {
    serde_json::json!({ "block_number": block_n })
}

#[derive(Debug, serde::Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Debug, serde::Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

fn rpc_call<T: DeserializeOwned>(client: &BlockingClient, rpc_url: &str, method: &str, params: serde_json::Value) -> Result<T> {
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .with_context(|| format!("RPC {method} request failed"))?
        .json::<RpcResponse<T>>()
        .with_context(|| format!("RPC {method} response parse failed"))?;

    if let Some(err) = response.error {
        return Err(anyhow!("RPC {method} error {}: {}", err.code, err.message));
    }

    response.result.ok_or_else(|| anyhow!("RPC {method} missing result"))
}

fn fetch_rpc_root(client: &BlockingClient, rpc_url: &str, block_n: u64) -> Result<Felt> {
    let block = rpc_call::<MaybePreConfirmedBlockWithTxHashes>(
        client,
        rpc_url,
        "starknet_getBlockWithTxHashes",
        serde_json::json!([block_id(block_n)]),
    )
    .with_context(|| format!("RPC getBlockWithTxHashes failed for block {}", block_n))?;

    match block {
        MaybePreConfirmedBlockWithTxHashes::Block(block) => Ok(block.new_root),
        MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => {
            bail!("RPC returned pre-confirmed block for number {}", block_n)
        }
    }
}

fn summarize_chain_tip(tip: StorageChainTip) -> String {
    match tip {
        StorageChainTip::Empty => "empty".to_string(),
        StorageChainTip::Confirmed(n) => format!("confirmed:{n}"),
        StorageChainTip::Preconfirmed { header, .. } => format!("preconfirmed:{}", header.block_number),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let db_path = resolve_db_path(&args.db_path)?;

    let storage = RocksDBStorage::open(&db_path, RocksDBConfig::default())
        .with_context(|| format!("Opening RocksDB at {}", db_path.display()))?;

    let latest_applied = storage.get_latest_applied_trie_update()?.unwrap_or(0);
    let chain_tip = storage.get_chain_tip()?;
    let stored_chain_info = storage.get_stored_chain_info()?;
    let state_root = storage.get_state_root_hash()?;

    println!("db_path={}", db_path.display());
    println!("chain_tip={}", summarize_chain_tip(chain_tip));
    if let Some(info) = stored_chain_info {
        println!("chain_name={}", info.chain_name);
        println!("chain_id={}", info.chain_id);
    }
    println!("latest_applied_trie_update={}", latest_applied);
    println!("state_root={}", format_felt(state_root));

    if let Some(rpc_url) = args.rpc_url {
        let client = BlockingClient::new();
        let rpc_root = fetch_rpc_root(&client, &rpc_url, latest_applied)?;
        println!("rpc_root={}", format_felt(rpc_root));
        if rpc_root != state_root {
            bail!(
                "Root mismatch at {}: db={}, rpc={}",
                latest_applied,
                format_felt(state_root),
                format_felt(rpc_root)
            );
        }
        println!("root_check=OK");
    }

    Ok(())
}
