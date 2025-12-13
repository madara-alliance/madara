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

    /// Update the compiled_class_hash_v2 (BLAKE hash) for existing classes (SNIP-34 migration).
    #[tracing::instrument(skip(self, migrations))]
    pub(crate) fn update_class_v2_hashes(&self, migrations: impl IntoIterator<Item = (Felt, Felt)>) -> Result<()> {
        let col = self.get_column(CLASS_INFO_COLUMN);
        let mut batch = WriteBatchWithTransaction::default();

        for (class_hash, blake_compiled_class_hash) in migrations {
            let Some(mut class_info_with_block) = self.get_class(&class_hash)? else {
                tracing::warn!("Cannot update v2 hash for class {class_hash:#x}: class not found in DB");
                continue;
            };

            match &mut class_info_with_block.class_info {
                mp_class::ClassInfo::Sierra(sierra_info) => {
                    sierra_info.compiled_class_hash_v2 = Some(blake_compiled_class_hash);
                }
                mp_class::ClassInfo::Legacy(_) => {
                    tracing::warn!("Cannot update v2 hash for class {class_hash:#x}: class is Legacy, not Sierra");
                    continue;
                }
            }

            batch.put_cf(&col, class_hash.to_bytes_be(), super::serialize(&class_info_with_block)?);
        }

        self.db.write_opt(batch, &self.writeopts)?;
        Ok(())
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
                self.db.write_opt(batch, &self.writeopts)?;
                anyhow::Ok(())
            },
        )?;

        converted_classes
            .iter()
            .filter_map(|converted_class| match converted_class {
                ConvertedClass::Sierra(sierra) => {
                    // Use canonical compiled_class_hash (v2 if present, else v1)
                    let canonical_hash = sierra.info.compiled_class_hash_v2.or(sierra.info.compiled_class_hash)?;
                    Some((canonical_hash, sierra.compiled.clone()))
                }
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
                    self.db.write_opt(batch, &self.writeopts)?;
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

    /// Revert class DB changes for a range of blocks.
    ///
    /// Handles three types of class changes:
    /// - Legacy classes (old_declared_contracts): Deleted entirely
    /// - Sierra classes (declared_classes): Class info and compiled class deleted
    /// - Migrated classes (migrated_compiled_classes): compiled_class_hash_v2 cleared
    #[tracing::instrument(skip(self, state_diffs))]
    pub(super) fn class_db_revert(&self, state_diffs: &[(u64, StateDiff)]) -> Result<()> {
        tracing::info!("🎓 REORG [class_db_revert]: Starting with {} state diffs", state_diffs.len());

        let class_info_col = self.get_column(CLASS_INFO_COLUMN);
        let class_compiled_col = self.get_column(CLASS_COMPILED_COLUMN);

        let mut batch = WriteBatchWithTransaction::default();
        let mut legacy_count = 0;
        let mut sierra_count = 0;
        let mut migrated_count = 0;

        for (_block_n, diff) in state_diffs {
            for class_hash in &diff.old_declared_contracts {
                batch.delete_cf(&class_info_col, class_hash.to_bytes_be());
                legacy_count += 1;
            }

            for declared_class in &diff.declared_classes {
                batch.delete_cf(&class_info_col, declared_class.class_hash.to_bytes_be());
                batch.delete_cf(&class_compiled_col, declared_class.compiled_class_hash.to_bytes_be());
                sierra_count += 1;
            }

            // Revert migrated classes: clear their compiled_class_hash_v2
            for migrated in &diff.migrated_compiled_classes {
                if let Some(mut class_info_with_block) = self.get_class(&migrated.class_hash)? {
                    if let mp_class::ClassInfo::Sierra(sierra_info) = &mut class_info_with_block.class_info {
                        sierra_info.compiled_class_hash_v2 = None;
                        batch.put_cf(
                            &class_info_col,
                            migrated.class_hash.to_bytes_be(),
                            super::serialize(&class_info_with_block)?,
                        );
                        migrated_count += 1;
                    }
                }
            }
        }

        self.db.write_opt(batch, &self.writeopts)?;

        if legacy_count > 0 || sierra_count > 0 || migrated_count > 0 {
            tracing::debug!(
                "Reverted classes: {} legacy, {} sierra, {} migrated",
                legacy_count,
                sierra_count,
                migrated_count
            );
        }

        Ok(())
    }
}
