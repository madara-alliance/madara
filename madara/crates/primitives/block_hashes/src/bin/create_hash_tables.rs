use clap::{Parser, ValueEnum};
use itertools::Itertools;
use mc_db::{stream::BlockStreamConfig, MadaraBackend, MadaraBackendConfig};
use mp_chain_config::{ChainConfig, StarknetVersion};
use mp_convert::ToFelt;
use starknet_types_core::{
    felt::Felt,
    hash::{Poseidon, StarkHash},
};
use std::{fs, io::Write};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
enum ChainId {
    Mainnet,
    Sepolia,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "/tmp/madaradb")]
    base_path: String,
    #[arg(long, default_value = "sepolia")]
    chain_id: ChainId,
    #[arg(long, default_value = "32")]
    chunk_size: u64,
    #[arg(long, default_value = "block_hashes.bin")]
    output: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let chain_config = match args.chain_id {
        ChainId::Mainnet => ChainConfig::starknet_mainnet(),
        ChainId::Sepolia => ChainConfig::starknet_sepolia(),
    };
    let backend = MadaraBackend::open(chain_config.into(), MadaraBackendConfig::new(args.base_path)).await?;

    let mut file = fs::OpenOptions::new().create(true).write(true).open(args.output)?;

    println!("Highest block_n in db: {:?}", backend.get_latest_block_n()?);

    let blocks_ite = backend
        .block_info_iterator(BlockStreamConfig::default().with_start(0u64).forward())
        .map(Result::unwrap)
        .take_while(|block_info| block_info.header.protocol_version < StarknetVersion::V0_13_2);

    for chunk in &blocks_ite.chunks(args.chunk_size as _) {
        let mut hash_chain = Felt::ZERO;
        let mut last_block_n = 0;
        for block_info in chunk {
            let old_hash = block_info.block_hash;
            let new_hash = block_info
                .header
                .compute_hash(backend.chain_config().chain_id.to_felt(), /* pre_v0_13_2_override */ true);

            let combined = Poseidon::hash(&old_hash, &new_hash);
            if last_block_n == 0 {
                hash_chain = combined;
            } else {
                hash_chain = Poseidon::hash(&combined, &hash_chain);
            }
            last_block_n = block_info.header.block_number;
        }

        let bytes = hash_chain.to_bytes_be();
        file.write(&bytes)?;
        println!("{} blocks done", last_block_n + 1);
    }
    println!("Done");

    Ok(())
}
