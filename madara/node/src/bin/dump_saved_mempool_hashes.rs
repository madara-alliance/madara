#![allow(clippy::print_stdout)]

use anyhow::{Context, Result};
use clap::Parser;
use mc_class_exec::config::NativeConfig;
use mc_db::{rocksdb::RocksDBConfig, MadaraBackend, MadaraBackendConfig};
use mp_chain_config::ChainConfig;
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Arc};

#[derive(Debug, Parser)]
struct Args {
    /// Madara base path that contains the db/ directory and version files.
    #[arg(long)]
    base_path: PathBuf,

    /// Chain config YAML used to open the database correctly.
    #[arg(long)]
    chain_config_path: PathBuf,

    /// Optional path to write the JSON manifest. Prints to stdout if omitted.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Optional expected count guard.
    #[arg(long)]
    expect_count: Option<usize>,
}

#[derive(Debug, Serialize)]
struct SavedMempoolHashesManifest {
    base_path: String,
    chain_config_path: String,
    count: usize,
    tx_hashes: Vec<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let chain_config = Arc::new(ChainConfig::from_yaml(&args.chain_config_path).context("Loading chain config")?);
    let backend = MadaraBackend::open_rocksdb(
        &args.base_path,
        chain_config,
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        Arc::new(NativeConfig::default()),
    )
    .context("Opening Madara backend")?;

    let mut hashes = backend
        .get_saved_mempool_transactions()
        .map(|res| res.map(|tx| tx.hash))
        .collect::<Result<Vec<_>, _>>()
        .context("Reading saved mempool transactions")?;
    hashes.sort();

    if let Some(expected) = args.expect_count {
        anyhow::ensure!(hashes.len() == expected, "Expected {expected} saved mempool txs, found {}", hashes.len());
    }

    let manifest = SavedMempoolHashesManifest {
        base_path: args.base_path.display().to_string(),
        chain_config_path: args.chain_config_path.display().to_string(),
        count: hashes.len(),
        tx_hashes: hashes.into_iter().map(|hash| format!("{hash:#x}")).collect(),
    };

    let json = serde_json::to_string_pretty(&manifest).context("Serializing manifest")?;
    if let Some(output) = args.output {
        fs::write(&output, format!("{json}\n")).with_context(|| format!("Writing manifest to {}", output.display()))?;
    } else {
        println!("{json}");
    }

    Ok(())
}
