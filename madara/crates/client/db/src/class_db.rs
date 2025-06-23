use crate::{
    db_block_id::{DbBlockIdResolvable, RawDbBlockId},
    Column, DatabaseExt, MadaraBackend, MadaraStorageError, WriteBatchWithTransaction, DB_UPDATES_BATCH_SIZE,
};
use mp_class::{ClassInfo, CompiledSierra, ConvertedClass, LegacyConvertedClass, SierraConvertedClass};
use rayon::{iter::ParallelIterator, slice::ParallelSlice};
use starknet_types_core::felt::Felt;
use std::sync::Arc;

const LAST_KEY: &[u8] = &[0xFF; 64];

type ClassHash = Felt;
type ClassHashCompiled = Felt;

#[derive(Debug, Default)]
pub(crate) struct ClassUpdates {
    class_info: std::collections::BTreeMap<ClassHash, ClassInfo>,
    class_compiled: std::collections::BTreeMap<ClassHashCompiled, std::sync::Arc<CompiledSierra>>,
}

impl ClassUpdates {
    pub fn from_converted_classes(
        backend: &MadaraBackend,
        converted_classes: &[ConvertedClass],
    ) -> Result<Self, MadaraStorageError> {
        let class_info = converted_classes.iter().try_fold(
            std::collections::BTreeMap::new(),
            |mut acc, converted_class| -> Result<_, MadaraStorageError> {
                let class_hash = converted_class.class_hash();
                if !backend.contains_class(&class_hash)? {
                    acc.insert(class_hash, converted_class.info());
                }
                Ok(acc)
            },
        )?;

        let class_compiled = converted_classes
            .iter()
            .filter_map(|converted_class| match converted_class {
                ConvertedClass::Sierra(sierra) => Some((sierra.info.compiled_class_hash, sierra.compiled.clone())),
                _ => None,
            })
            .fold(std::collections::BTreeMap::new(), |mut acc, (class_hash, compiled)| {
                acc.insert(class_hash, compiled);
                acc
            });

        Ok(Self { class_info, class_compiled })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ClassInfoWithBlockNumber {
    class_info: ClassInfo,
    block_id: RawDbBlockId,
}

impl MadaraBackend {
    #[tracing::instrument(skip(self, key), fields(module = "ClassDB"))]
    fn class_db_get_encoded_kv<V: serde::de::DeserializeOwned>(
        &self,
        is_pending: bool,
        key: &Felt,
        nonpending_col: Column,
    ) -> Result<Option<V>, MadaraStorageError> {
        // todo: smallint here to avoid alloc
        tracing::trace!("class db get encoded kv, key={key:#x}");
        let key_encoded = bincode::serialize(key)?;

        tracing::trace!("class db get encoded kv, state is not pending");

        let col = self.db.get_column(nonpending_col);
        let Some(val) = self.db.get_pinned_cf(&col, &key_encoded)? else { return Ok(None) };
        let val = bincode::deserialize(&val)?;

        Ok(Some(val))
    }

    #[tracing::instrument(skip(self, id, class_hash), fields(module = "ClassDB"))]
    pub fn get_class_info(
        &self,
        id: &impl DbBlockIdResolvable,
        class_hash: &Felt,
    ) -> Result<Option<ClassInfo>, MadaraStorageError> {
        let Some(requested_id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        tracing::debug!("get class info {requested_id:?} {class_hash:#x}");

        let is_pending = requested_id.is_pending();
        if is_pending {
            if let Some(class_info) = self.latest_pending_block().classes.class_info.get(class_hash) {
                return Ok(Some(class_info.clone()));
            }
        }

        let Some(info) =
            self.class_db_get_encoded_kv::<ClassInfoWithBlockNumber>(is_pending, class_hash, Column::ClassInfo)?
        else {
            tracing::debug!("no class info");
            return Ok(None);
        };

        tracing::debug!("class info got {:?}", info.block_id);

        let valid = match (requested_id, info.block_id) {
            (RawDbBlockId::Pending, _) => true,
            (RawDbBlockId::Number(block_n), RawDbBlockId::Number(real_block_n)) => real_block_n <= block_n,
            _ => false,
        };
        if !valid {
            tracing::debug!("rejected {:?}", (requested_id, info.block_id));
            return Ok(None);
        }
        tracing::debug!("class db get class info, state is valid");

        Ok(Some(info.class_info))
    }

    #[tracing::instrument(skip(self), fields(module = "ClassDB"))]
    pub fn contains_class(&self, class_hash: &Felt) -> Result<bool, MadaraStorageError> {
        let col = self.db.get_column(Column::ClassInfo);
        let key_encoded = bincode::serialize(class_hash)?;
        Ok(self.db.get_pinned_cf(&col, &key_encoded)?.is_some())
    }

    #[tracing::instrument(skip(self, id, compiled_class_hash), fields(module = "ClassDB"))]
    pub fn get_sierra_compiled(
        &self,
        id: &impl DbBlockIdResolvable,
        compiled_class_hash: &Felt,
    ) -> Result<Option<CompiledSierra>, MadaraStorageError> {
        let Some(requested_id) = id.resolve_db_block_id(self)? else { return Ok(None) };

        tracing::trace!("get sierra compiled {requested_id:?} {compiled_class_hash:#x}");

        let is_pending = requested_id.is_pending();
        if is_pending {
            if let Some(compiled_class) = self.latest_pending_block().classes.class_compiled.get(compiled_class_hash) {
                return Ok(Some(compiled_class.as_ref().clone()));
            }
        }

        let Some(compiled) =
            self.class_db_get_encoded_kv::<CompiledSierra>(is_pending, compiled_class_hash, Column::ClassCompiled)?
        else {
            return Ok(None);
        };

        Ok(Some(compiled))
    }

    /// Get class info + sierra compiled when it's a sierra class.
    // Note/TODO: "ConvertedClass" is the name of the type that has info + sierra compiled, and it is used for blockifier
    // convertion & storage. We should rename it, as this feels like undecipherable madara-specific jargon at this point.
    #[tracing::instrument(skip(self, id), fields(module = "ClassDB"))]
    pub fn get_converted_class(
        &self,
        id: &impl DbBlockIdResolvable,
        class_hash: &Felt,
    ) -> Result<Option<ConvertedClass>, MadaraStorageError> {
        let Some(id) = id.resolve_db_block_id(self)? else {
            // Block not found
            return Ok(None);
        };

        let Some(class_info) = self.get_class_info(&id, class_hash)? else {
            // No class found.
            return Ok(None);
        };

        match class_info {
            ClassInfo::Sierra(info) => {
                let compiled_class_hash = info.compiled_class_hash;
                let compiled_class = self
                    .get_sierra_compiled(&id, &info.compiled_class_hash)?
                    .ok_or(MadaraStorageError::MissingCompiledClass { class_hash: *class_hash, compiled_class_hash })?;
                Ok(Some(ConvertedClass::Sierra(SierraConvertedClass {
                    class_hash: *class_hash,
                    info,
                    // TODO(perf): we should do global memoization for these Arcs.
                    compiled: Arc::new(compiled_class),
                })))
            }
            ClassInfo::Legacy(info) => {
                Ok(Some(ConvertedClass::Legacy(LegacyConvertedClass { class_hash: *class_hash, info })))
            }
        }
    }

    /// NB: This functions needs to run on the rayon thread pool
    #[tracing::instrument(skip(self, converted_classes, col_info, col_compiled), fields(module = "ClassDB"))]
    pub(crate) fn store_classes(
        &self,
        block_id: RawDbBlockId,
        converted_classes: &[ConvertedClass],
        col_info: Column,
        col_compiled: Column,
    ) -> Result<(), MadaraStorageError> {
        tracing::trace!(
            "Store class {block_id:?} {:?}",
            converted_classes.iter().map(|c| c.class_hash()).collect::<Vec<_>>()
        );

        converted_classes.par_chunks(DB_UPDATES_BATCH_SIZE).try_for_each_init(
            || self.db.get_column(col_info),
            |col, chunk| {
                let mut batch = WriteBatchWithTransaction::default();
                for converted_class in chunk {
                    let class_hash = converted_class.class_hash();
                    let key_bin = bincode::serialize(&class_hash)?;
                    // this is a patch because some legacy classes are declared multiple times
                    if !self.contains_class(&class_hash)? {
                        // TODO: find a way to avoid this allocation
                        batch.put_cf(
                            col,
                            &key_bin,
                            bincode::serialize(&ClassInfoWithBlockNumber {
                                class_info: converted_class.info(),
                                block_id,
                            })?,
                        );
                    }
                }
                self.db.write_opt(batch, &self.writeopts_no_wal)?;
                Ok::<_, MadaraStorageError>(())
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
                || self.db.get_column(col_compiled),
                |col, chunk| {
                    let mut batch = WriteBatchWithTransaction::default();
                    for (key, value) in chunk {
                        tracing::trace!("Class compiled store key={key:#x}");
                        let key_bin = bincode::serialize(key)?;
                        // TODO: find a way to avoid this allocation
                        batch.put_cf(col, &key_bin, bincode::serialize(&value)?);
                    }
                    self.db.write_opt(batch, &self.writeopts_no_wal)?;
                    Ok::<_, MadaraStorageError>(())
                },
            )?;

        Ok(())
    }

    /// NB: This functions needs to run on the rayon thread pool
    #[tracing::instrument(skip(self, converted_classes), fields(module = "ClassDB"))]
    pub fn store_block_classes(
        &self,
        block_number: u64,
        converted_classes: &[ConvertedClass],
    ) -> Result<(), MadaraStorageError> {
        tracing::debug!(
            "Storing classes block_n={block_number} {:?}",
            converted_classes.iter().map(|c| format!("{:#x}", c.class_hash())).collect::<Vec<_>>()
        );
        self.store_classes(
            RawDbBlockId::Number(block_number),
            converted_classes,
            Column::ClassInfo,
            Column::ClassCompiled,
        )
    }
}
