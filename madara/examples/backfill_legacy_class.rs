use anyhow::{bail, Context};
use clap::Parser;
use mc_class_exec::config::NativeConfig;
use mc_db::{rocksdb::RocksDBConfig, MadaraBackend, MadaraBackendConfig, MadaraStorageRead};
use mc_gateway_client::{BlockId, GatewayProvider};
use mp_chain_config::ChainConfig;
use mp_class::{ClassInfo, ConvertedClass, LegacyClassInfo, LegacyConvertedClass};
use starknet_types_core::felt::Felt;
use std::{path::PathBuf, sync::Arc};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "/mnt/data")]
    db_path: PathBuf,
    #[arg(long)]
    block_number: u64,
    #[arg(long)]
    class_hash: String,
}

fn setup_rayon_threadpool() -> anyhow::Result<()> {
    let available_parallelism = std::thread::available_parallelism()?;
    rayon::ThreadPoolBuilder::new()
        .thread_name(|thread_index| format!("rayon-{thread_index}"))
        .num_threads(available_parallelism.get())
        .build_global()?;
    Ok(())
}

fn raise_fdlimit() {
    let _ = fdlimit::raise_fd_limit();
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).try_init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    setup_rayon_threadpool()?;
    raise_fdlimit();

    let args = Args::parse();
    let class_hash =
        Felt::from_hex(&args.class_hash).with_context(|| format!("Parsing class hash {}", args.class_hash))?;

    let native_builder = NativeConfig::builder();
    mc_class_exec::init_compilation_semaphore(native_builder.max_concurrent_compilations());
    let cairo_native_config = Arc::new(native_builder.build());

    let backend = MadaraBackend::open_rocksdb(
        &args.db_path,
        Arc::new(ChainConfig::starknet_mainnet()),
        MadaraBackendConfig::default(),
        RocksDBConfig::default(),
        cairo_native_config,
    )
    .context("Opening Madara backend")?;

    if backend.db.get_class(&class_hash)?.is_some() {
        println!("Class {class_hash:#x} is already present in the database");
        return Ok(());
    }

    let provider = GatewayProvider::starknet_alpha_mainnet();
    let class = provider
        .get_class_by_hash(class_hash, BlockId::Number(args.block_number))
        .await
        .with_context(|| format!("Fetching legacy class {class_hash:#x} at block {}", args.block_number))?;

    let converted = match class {
        mp_class::ContractClass::Legacy(contract_class) => {
            ConvertedClass::Legacy(LegacyConvertedClass { class_hash, info: LegacyClassInfo { contract_class } })
        }
        mp_class::ContractClass::Sierra(_) => {
            bail!("Class {class_hash:#x} is Sierra, but this helper only backfills legacy classes")
        }
    };

    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()?
        .install(|| backend.write_access().write_classes(args.block_number, &[converted]))
        .context("Writing legacy class into RocksDB")?;

    let Some(class_info) = backend.view_on_latest_confirmed().get_class_info(&class_hash)? else {
        bail!("Class {class_hash:#x} was written but is not visible from latest confirmed state");
    };

    match class_info {
        ClassInfo::Legacy(_) => {
            println!("Backfilled legacy class {class_hash:#x} into {}", args.db_path.display());
        }
        ClassInfo::Sierra(_) => bail!("Class {class_hash:#x} resolved to Sierra after legacy backfill"),
    }

    Ok(())
}
