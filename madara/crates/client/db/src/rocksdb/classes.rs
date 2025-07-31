use crate::{
    prelude::*, rocksdb::{Column, RocksDBStorageInner, WriteBatchWithTransaction, DB_UPDATES_BATCH_SIZE}, storage::{ClassInfoWithBlockN, CompiledSierraWithBlockN}
};
use mp_class::ConvertedClass;
use mp_convert::Felt;
use rayon::{iter::ParallelIterator, slice::ParallelSlice};

/// <class_hash 32 bytes> => bincode(class_info)
pub const CLASS_INFO_COLUMN: Column = Column::new("class_info").set_point_lookup();
/// <compiled_class_hash 32 bytes> => bincode(class_info)
pub const CLASS_COMPILED_COLUMN: Column = Column::new("class_compiled").set_point_lookup();

impl RocksDBStorageInner {
    #[tracing::instrument(skip(self))]
    pub(super) fn get_class(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(CLASS_INFO_COLUMN), &class_hash.to_bytes_be())? else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_class_compiled(
        &self,
        compiled_class_hash: &Felt,
    ) -> Result<Option<CompiledSierraWithBlockN>> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(CLASS_COMPILED_COLUMN), &compiled_class_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn contains_class(&self, class_hash: &Felt) -> Result<bool> {
        Ok(self.db.get_pinned_cf(&self.get_column(CLASS_INFO_COLUMN), &class_hash.to_bytes_be())?.is_some())
    }

    #[tracing::instrument(skip(self, converted_classes))]
    pub(crate) fn store_classes(&self, block_n: u64, converted_classes: &[ConvertedClass]) -> Result<()> {
        converted_classes.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.get_column(CLASS_INFO_COLUMN),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for converted_class in chunk {
                    // this is a patch because some legacy classes are declared multiple times
                    if !self.contains_class(&converted_class.class_hash())? {
                        // TODO: find a way to avoid this allocation
                        batch.put_cf(
                            col,
                            &converted_class.class_hash().to_bytes_be(),
                            bincode::serialize(&ClassInfoWithBlockN { class_info: converted_class.info(), block_n })?,
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
                    for (key, value) in chunk {
                        batch.put_cf(col, &key.to_bytes_be(), bincode::serialize(&value)?);
                    }
                    self.db.write_opt(batch, &self.writeopts_no_wal)?;
                    anyhow::Ok(())
                },
            )?;

        Ok(())
    }
}
