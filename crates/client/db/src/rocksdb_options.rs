//! # Rocksdb options
//!
//! We configure the rocksdb database in a very specific way. We want to guarantee a few things:
//!
//! ## Fault tolerance
//!
//! The madara node can be stopped at any time, and it should be able to recover when restarting.
//! In particular, in case of a power/hardware failure, the database should not be corrupted beyond repair. We cannot
//! run code that would allow us to gracefully shutdown in these cases.
//!
//! The way rocksdb ensures this is usually by using a WAL (write-ahead log) and doing a 2PC (two-phase commit) under
//! the hood. There is however another way to achieve fault tolerance with rocksdb, which is by enabling the [atomic flush]
//! option.
//!
//! ## Consistent view of the database
//!
//! The usual way it is achieved is by batching all of the changes into a single batch or a single rocksdb transaction so
//! that they are applied to the database atomically.
//!
//! ## Multithreaded commit to the database
//!
//! This is main reason we differ from the default rocksdb configuration: rocksdb Transaction objects are not thread safe,
//! and committing a transaction also isn't multithreaded at all. This means that if we want to write a whole block into the
//! database, we would have to put all of the changes into a single WriteBatch and apply it all at once in a single-threaded
//! fashion. Rocksdb has [WriteUnprepared transactions] which would help us avoid buffering all the changes before applying them,
//! but the transaction object still cannot be passed to other threads, so that wouldn't work either.
//!
//! The way we work around that is by:
//! - enabling atomic flushing
//! - disabling the WAL, as we don't need it with atomic flushing
//! - disabling auto flushing, so that a block cannot be written into the db half way
//! - do not use any transactions, do all the writes directly to the database.
//! - use explicit [Snapshot]s for reads: all reads use the latest snapshot; and between every block we do a new snapshot and drop the old one.
//!   This ensures we don't read the database when a block has been half-way written
//!
//! That last point isn't yet implemented, as we need some changes to the rust rocksdb bindings, see [rust-rocksdb#937]. No issue has yet been
//! found with this as our db is almost only append-only - but this should nonetheless be fixed. (FIXME)
//!
//! This configuration makes a lot of sense because we making a blockchain node. All of our db writes are very big, all or nothing and infrequent.
//! Except for very few things: there are places where we still use the WAL, such as when we write mempool transactions to the database. This
//! ensures that they are not dropped when restarting the node between two blocks.
//!
//! [atomic flush]: https://github.com/facebook/rocksdb/wiki/Atomic-flush
//! [WriteUnprepared transactions]: https://github.com/facebook/rocksdb/wiki/WriteUnprepared-Transactions
//! [Snapshot]: https://github.com/facebook/rocksdb/wiki/Snapshot
//! [rust-rocksdb#937]: https://github.com/rust-rocksdb/rust-rocksdb/issues/937

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

    // See module documentation.
    options.set_atomic_flush(true);

    // By default rocksdb will spam a lot of info about every column very regularily, making huge files that take
    // multiple gigabytes. Using log level warn, this info will not be printed to the log file. In addition,
    // we limit the size and number of files.
    options.set_max_log_file_size(10 * MiB);
    options.set_keep_log_file_num(3);
    options.set_log_level(rocksdb::LogLevel::Warn);

    // Max number of open files for rocksdb. This number has been chosen a bit arbitrarily, but it should low enough
    // to leave a bunch of available file descriptors for peer-to-peer tcp sockets.
    // NOTE(cchudant): I do not believe setting this limit would yield much perf, but this has not been tested.
    options.set_max_open_files(2048);

    // Concurrency options
    let cores = std::thread::available_parallelism().map(|e| e.get() as i32).unwrap_or(1);
    options.increase_parallelism(cores);
    options.set_max_background_jobs(cores);
    options.set_max_subcompactions(cores as _);
    let mut env = Env::new().context("Creating rocksdb env")?;
    // env.set_high_priority_background_threads(cores); // flushes - our flushes are manual so this option is not useful.
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

        // We use universal-style compaction and not level-style compaction because the compaction can't keep up with our
        // flushes otherwise, and we end up with more and more SST files that never get compacted.
        // See https://github.com/facebook/rocksdb/wiki/Universal-Compaction.

        // NOTE(perf,cchudant): these numbers were eyeballed, they could be refined. We should also try the point-lookup
        // column option for columns where we don't need iterators.

        options.set_compression_type(DBCompressionType::Zstd);
        match self {
            Column::BlockNToBlockInfo | Column::BlockNToBlockInner => {
                let memtable_memory_budget = 1 * GiB;
                options.optimize_universal_style_compaction(memtable_memory_budget);
            }
            _ => {
                let memtable_memory_budget = 100 * MiB;
                options.optimize_universal_style_compaction(memtable_memory_budget);
            }
        }
        options
    }
}
