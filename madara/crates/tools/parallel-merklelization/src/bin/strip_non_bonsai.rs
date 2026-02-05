use anyhow::{Context, Result};
use clap::Parser;
use rocksdb::{IteratorMode, Options, WriteBatch, DB};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "strip-non-bonsai", about = "Clear non-bonsai columns in a Madara RocksDB")]
struct Args {
    #[arg(long, value_name = "PATH")]
    db_path: PathBuf,
}

const KEEP_COLUMNS: &[&str] = &[
    "default",
    "meta",
    "preconfirmed",
    "bonsai_contract_flat",
    "bonsai_contract_trie",
    "bonsai_contract_log",
    "bonsai_contract_storage_flat",
    "bonsai_contract_storage_trie",
    "bonsai_contract_storage_log",
    "bonsai_class_flat",
    "bonsai_class_trie",
    "bonsai_class_log",
];

fn main() -> Result<()> {
    let args = Args::parse();
    let db_path = resolve_db_path(&args.db_path)?;

    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);

    let cf_names =
        DB::list_cf(&opts, &db_path).with_context(|| format!("Listing column families at {}", db_path.display()))?;
    let db = DB::open_cf(&opts, &db_path, &cf_names).with_context(|| format!("Opening DB at {}", db_path.display()))?;

    let keep: HashSet<&str> = KEEP_COLUMNS.iter().copied().collect();

    for cf_name in &cf_names {
        if keep.contains(cf_name.as_str()) {
            continue;
        }

        let Some(cf) = db.cf_handle(cf_name) else {
            continue;
        };

        let mut keys: Vec<Vec<u8>> = Vec::new();
        for item in db.iterator_cf(cf, IteratorMode::Start) {
            let (key, _value) = item?;
            keys.push(key.to_vec());
        }

        if keys.is_empty() {
            continue;
        }

        let mut batch = WriteBatch::default();
        let mut count: usize = 0;
        for key in keys {
            batch.delete_cf(cf, key);
            count += 1;
            if count % 10_000 == 0 {
                db.write(batch)?;
                batch = WriteBatch::default();
            }
        }
        if count % 10_000 != 0 {
            db.write(batch)?;
        }

        db.flush_cf(cf)?;
        db.compact_range_cf(cf, None::<&[u8]>, None::<&[u8]>);
        println!("cleared {} keys from column {}", count, cf_name);
    }

    Ok(())
}

fn resolve_db_path(input: &Path) -> Result<PathBuf> {
    if input.join("CURRENT").exists() {
        return Ok(input.to_path_buf());
    }
    let nested = input.join("db");
    if nested.join("CURRENT").exists() {
        return Ok(nested);
    }
    Ok(input.to_path_buf())
}
