// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0
// This file is part of Frontier.
//
// Copyright (c) 2020-2022 Parity Technologies (UK) Ltd.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

pub mod parity_db_adapter;
pub mod rock_db_adapter;

use std::path::Path;
use std::sync::Arc;

use kvdb::KeyValueDB;
use sp_database::Database;

use crate::{DatabaseSettings, DatabaseSource, DbHash};

#[allow(clippy::type_complexity)]
pub(crate) fn open_database(
    config: &DatabaseSettings,
) -> Result<(Arc<dyn KeyValueDB>, Arc<dyn Database<DbHash>>), String> {
    let dbs: (Arc<dyn KeyValueDB>, Arc<dyn Database<DbHash>>) = match &config.source {
        // DatabaseSource::ParityDb { path } => open_parity_db(path).expect("Failed to open parity db"),
        DatabaseSource::RocksDb { path, .. } => {
            let dbs = open_kvdb_rocksdb(path, true)?;
            (dbs.0, dbs.1)
        }
        DatabaseSource::Auto { paritydb_path, rocksdb_path, .. } => match open_kvdb_rocksdb(rocksdb_path, false) {
            Ok(_) => {
                let dbs = open_kvdb_rocksdb(paritydb_path, true)?;
                (dbs.0, dbs.1)
            }
            Err(_) => Err("Missing feature flags `parity-db`".to_string())?,
        },
        _ => return Err("Missing feature flags `parity-db`".to_string()),
    };
    Ok(dbs)
}

#[allow(clippy::type_complexity)]
pub fn open_kvdb_rocksdb(
    path: &Path,
    create: bool,
) -> Result<(Arc<dyn KeyValueDB>, Arc<dyn Database<DbHash>>), String> {
    let mut db_config = kvdb_rocksdb::DatabaseConfig::with_columns(crate::columns::NUM_COLUMNS);
    db_config.create_if_missing = create;

    let db_kvdb = kvdb_rocksdb::Database::open(&db_config, path).map_err(|err| format!("{}", err))?;
    let x = Arc::new(db_kvdb);
    let y = unsafe { std::mem::transmute::<_, Arc<rock_db_adapter::DbAdapter>>(x.clone()) };

    Ok((x, y))
}
