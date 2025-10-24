use crate::{
    prelude::*,
    rocksdb::{Column, RocksDBStorageInner, WriteBatchWithTransaction, DB_UPDATES_BATCH_SIZE},
    storage::{ClassInfoWithBlockN, CompiledSierraWithBlockN},
};
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::{DeclaredClassCompiledClass, StateDiff};
use rayon::{iter::ParallelIterator, slice::ParallelSlice};

/// <class_hash 32 bytes> => bincode(class_info)
pub const CLASS_INFO_COLUMN: Column = Column::new("class_info").set_point_lookup();
/// <compiled_class_hash 32 bytes> => bincode(class_info)
pub const CLASS_COMPILED_COLUMN: Column = Column::new("class_compiled").set_point_lookup();

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self))]
    pub(super) fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(CLASS_INFO_COLUMN), class_hash.to_bytes_be())? else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_class_compiled(&self, compiled_class_hash: &Felt) -> Result<Option<CompiledSierraWithBlockN>> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(CLASS_COMPILED_COLUMN), compiled_class_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(super::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn contains_class(&self, class_hash: &Felt) -> Result<bool> {
        Ok(self.db.get_pinned_cf(&self.get_column(CLASS_INFO_COLUMN), class_hash.to_bytes_be())?.is_some())
    }

    #[tracing::instrument(skip(self, converted_classes))]
    pub(crate) fn store_classes(&self, block_number: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        converted_classes.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.get_column(CLASS_INFO_COLUMN),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for converted_class in chunk {
                    // this is a patch because some legacy classes are declared multiple times
                    if !self.contains_class(converted_class.class_hash())? {
                        batch.put_cf(
                            col,
                            converted_class.class_hash().to_bytes_be(),
                            super::serialize(&ClassInfoWithBlockN {
                                class_info: converted_class.info(),
                                block_number,
                            })?,
                        );
                    }
                }
                self.db.write_opt(batch, &self.writeopts_no_wal)?;
                anyhow::Ok(())
            },
        )?;

        converted_classes
            .iter()
            .filter_map(|converted_class| match converted_class {
                ConvertedClass::Sierra(sierra) => Some((sierra.info.compiled_class_hash, sierra.compiled.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
            .par_chunks(DB_UPDATES_BATCH_SIZE)
            .try_for_each_init(
                || self.get_column(CLASS_COMPILED_COLUMN),
                |col, chunk| {
                    let mut batch = WriteBatchWithTransaction::default();
                    for (key, compiled_sierra) in chunk {
                        batch.put_cf(
                            col,
                            key.to_bytes_be(),
                            super::serialize(&CompiledSierraWithBlockN {
                                block_number,
                                compiled_sierra: compiled_sierra.clone(),
                            })?,
                        );
                    }
                    self.db.write_opt(batch, &self.writeopts_no_wal)?;
                    anyhow::Ok(())
                },
            )?;

        Ok(())
    }

    pub(crate) fn classes_remove(
        &self,
        classes: impl IntoIterator<Item = (Felt, DeclaredClassCompiledClass)>,
        batch: &mut WriteBatchWithTransaction,
    ) -> Result<()> {
        let class_info_col = self.get_column(CLASS_INFO_COLUMN);
        let class_compiled_col = self.get_column(CLASS_COMPILED_COLUMN);

        for (class_hash, compiled_class_hash) in classes {
            batch.delete_cf(&class_info_col, class_hash.to_bytes_be());
            if let DeclaredClassCompiledClass::Sierra(compiled_class_hash) = compiled_class_hash {
                batch.delete_cf(&class_compiled_col, compiled_class_hash.to_bytes_be());
            }
        }

        Ok(())
    }

    /// Revert items in the class db.
    ///
    /// `state_diffs` should be a Vec of tuples containing the block number and the entire StateDiff
    /// to be reverted in that block.
    ///
    /// **Warning:** While not enforced, the following should be true:
    ///  * Each `StateDiff` should include all deployed classes for its block
    ///  * `state_diffs` should form a contiguous range of blocks
    ///  * that range should end with the current blockchain tip
    ///
    /// If this isn't the case, the db could end up storing classes that aren't canonically
    /// deployed.
    #[tracing::instrument(skip(self, state_diffs))]
    pub(super) fn class_db_revert(&self, state_diffs: &[(u64, StateDiff)]) -> Result<()> {
        tracing::info!("🎓 REORG [class_db_revert]: Starting with {} state diffs", state_diffs.len());

        let class_info_col = self.get_column(CLASS_INFO_COLUMN);
        let class_compiled_col = self.get_column(CLASS_COMPILED_COLUMN);

        let mut batch = WriteBatchWithTransaction::default();
        let mut total_old_classes = 0;
        let mut total_classes = 0;

        for (block_n, diff) in state_diffs {
            tracing::debug!(
                "🎓 REORG [class_db_revert]: Processing block {} with {} legacy classes, {} sierra classes",
                block_n,
                diff.old_declared_contracts.len(),
                diff.declared_classes.len()
            );
            for class_hash in &diff.old_declared_contracts {
                batch.delete_cf(&class_info_col, class_hash.to_bytes_be());
                total_old_classes += 1;
            }
            for declared_class in &diff.declared_classes {
                batch.delete_cf(&class_info_col, declared_class.class_hash.to_bytes_be());
                batch.delete_cf(&class_compiled_col, declared_class.compiled_class_hash.to_bytes_be());
                total_classes += 1;
            }
        }

        tracing::info!(
            "🎓 REORG [class_db_revert]: Removing {} legacy classes and {} sierra classes",
            total_old_classes,
            total_classes
        );

        self.db.write_opt(batch, &self.writeopts_no_wal)?;

        tracing::info!("✅ REORG [class_db_revert]: Successfully removed all declared classes");

        Ok(())
    }
}
