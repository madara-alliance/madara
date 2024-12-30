#![allow(clippy::identity_op)] // allow 1 * MiB
#![allow(non_upper_case_globals)] // allow KiB/MiB/GiB names

use crate::{contract_db, Column};
use anyhow::{Context, Result};
use rocksdb::{DBCompressionType, Env, Options, SliceTransform};

const KiB: usize = 1024;
const MiB: usize = 1024 * KiB;
const GiB: usize = 1024 * MiB;

pub fn rocksdb_global_options() -> Result<Options> {
    let mut options = Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    options.increase_parallelism(cores);
    options.set_max_background_jobs(cores);

    options.set_atomic_flush(true);
    options.set_max_subcompactions(cores as _);

    options.set_max_log_file_size(10 * MiB);
    options.set_max_open_files(2048);
    options.set_keep_log_file_num(3);
    options.set_log_level(rocksdb::LogLevel::Warn);

    let mut env = Env::new().context("Creating rocksdb env")?;
    // env.set_high_priority_background_threads(cores); // flushes
    env.set_low_priority_background_threads(cores); // compaction

    options.set_env(&env);

    Ok(options)
}

impl Column {
    /// Per column rocksdb options, like memory budget, compaction profiles, block sizes for hdd/sdd
    /// etc.
    pub(crate) fn rocksdb_options(&self) -> Options {
        let mut options = Options::default();

        match self {
            Column::ContractStorage => {
                options.set_prefix_extractor(SliceTransform::create_fixed_prefix(
                    contract_db::CONTRACT_STORAGE_PREFIX_EXTRACTOR,
                ));
            }
            Column::ContractToClassHashes => {
                options.set_prefix_extractor(SliceTransform::create_fixed_prefix(
                    contract_db::CONTRACT_CLASS_HASH_PREFIX_EXTRACTOR,
                ));
            }
            Column::ContractToNonces => {
                options.set_prefix_extractor(SliceTransform::create_fixed_prefix(
                    contract_db::CONTRACT_NONCES_PREFIX_EXTRACTOR,
                ));
            }
            _ => {}
        }

        options.set_compression_type(DBCompressionType::Zstd);
        match self {
            Column::BlockNToBlockInfo | Column::BlockNToBlockInner => {
                options.optimize_universal_style_compaction(1 * GiB);
            }
            _ => {
                options.optimize_universal_style_compaction(100 * MiB);
            }
        }
        options
    }
}
