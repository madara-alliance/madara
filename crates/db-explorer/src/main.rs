use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use env_logger::Env;
use mc_db::{Column, DatabaseExt, DeoxysBackend};
use rocksdb::IteratorMode;

#[derive(Parser, Debug)]
#[command()]
struct Args {
    #[arg(long)]
    path: String,

    #[arg(long)]
    backup_path: Option<String>,
}

fn main() -> Result<()> {
    env_logger::init_from_env(Env::default().default_filter_or("info"));

    let args = Args::parse();
    log::info!("Hello");

    let db_path = PathBuf::from(args.path).join("chains/starknet");

    DeoxysBackend::open(&db_path, args.backup_path.map(Into::into), false, false).context("opening db")?;

    log::info!("Running db bench");
    run_db_bench()?;

    Ok(())
}

pub fn run_db_bench() -> Result<()> {
    // Block number

    let bytes = bench_db_column(Column::BlockHashToNumber)?;
    log::info!("block number: {bytes} bytes");

    // Block hash

    let bytes = bench_db_column(Column::BlockNumberToHash)?;
    log::info!("block hash: {bytes} bytes");

    // Contract class and abi

    let bytes = bench_db_column(Column::ContractClassData)?;
    log::info!("contract class data: {bytes} bytes");

    // Contract class hash & nonce

    let bytes = bench_db_column(Column::ContractData)?;
    log::info!("contract data: {bytes} bytes");

    // Compiled class hashes

    let bytes = bench_db_column(Column::ContractClassHashes)?;
    log::info!("contract class hashes: {bytes} bytes");

    // State diffs

    let bytes = bench_db_column(Column::BlockStateDiff)?;
    log::info!("State diffs: {bytes} bytes");

    // Bonsai contract

    let bytes = bench_db_column(Column::BonsaiContractsTrie)?;
    log::info!("bonsai contract trie: {bytes} bytes");

    let bytes = bench_db_column(Column::BonsaiContractsFlat)?;
    log::info!("bonsai contract flat: {bytes} bytes");

    let bytes = bench_db_column(Column::BonsaiContractsLog)?;
    log::info!("bonsai contract log: {bytes} bytes");

    // Bonsai contract storage

    let bytes = bench_db_column(Column::BonsaiContractsStorageTrie)?;
    log::info!("bonsai contract storage trie: {bytes} bytes");

    let bytes = bench_db_column(Column::BonsaiContractsStorageFlat)?;
    log::info!("bonsai contract storage flat: {bytes} bytes");

    let bytes = bench_db_column(Column::BonsaiContractsStorageLog)?;
    log::info!("bonsai contract storage log: {bytes} bytes");

    let bytes = bench_db_column(Column::ContractStorage)?;
    log::info!("contract storage log: {bytes} bytes");

    // Bonsai class

    let bytes = bench_db_column(Column::BonsaiClassesTrie)?;
    log::info!("bonsai class trie: {bytes} bytes");

    let bytes = bench_db_column(Column::BonsaiClassesFlat)?;
    log::info!("bonsai class flat: {bytes} bytes");

    let bytes = bench_db_column(Column::BonsaiClassesLog)?;
    log::info!("bonsai class log: {bytes} bytes");

    Ok(())
}

fn bench_db_column(column: Column) -> Result<usize> {
    let db = DeoxysBackend::expose_db();
    let handle = db.get_column(column);

    let mut bytes = 0;
    for cursor in db.iterator_cf(&handle, IteratorMode::Start) {
        let (key, value) = cursor.context("db iterator")?;

        bytes += key.len() + value.len();
    }

    Ok(bytes)
}
