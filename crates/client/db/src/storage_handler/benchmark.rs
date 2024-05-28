use rocksdb::IteratorMode;

use crate::{Column, DatabaseExt, DbError, DeoxysBackend};

pub fn run_db_bench() -> Result<(), DbError> {
    for column in Column::iter() {
        let bytes = bench_db_column(column)?;
        log::info!("{}: {} bytes", column, bytes);
    }

    Ok(())
}

fn bench_db_column(column: Column) -> Result<usize, DbError> {
    let db = DeoxysBackend::expose_db();
    let handle = db.get_column(column);

    let mut bytes = 0;
    for cursor in db.iterator_cf(&handle, IteratorMode::Start) {
        let (key, value) = cursor?;

        bytes += key.len() + value.len();
    }

    Ok(bytes)
}
