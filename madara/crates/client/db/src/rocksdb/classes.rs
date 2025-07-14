use crate::{
    db::{ClassInfoWithBlockN, CompiledSierraWithBlockN},
    rocksdb::{Column, RocksDBBackend},
    MadaraStorageError,
};
use mp_convert::Felt;

/// <class_hash 32 bytes> => bincode(class_info)
pub(crate) const CLASS_INFO_COLUMN: &Column = &Column::new("class_info").set_point_lookup();
/// <compiled_class_hash 32 bytes> => bincode(class_info)
pub(crate) const CLASS_COMPILED_COLUMN: &Column = &Column::new("class_cmpiled").set_point_lookup();

impl RocksDBBackend {
    #[tracing::instrument(skip(self))]
    pub(super) fn get_class_impl(&self, class_hash: &Felt) -> Result<Option<ClassInfoWithBlockN>, MadaraStorageError> {
        let Some(res) = self.db.get_pinned_cf(&self.get_column(CLASS_INFO_COLUMN), &class_hash.to_bytes_be())? else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }

    #[tracing::instrument(skip(self))]
    pub(super) fn get_class_compiled_impl(
        &self,
        compiled_class_hash: &Felt,
    ) -> Result<Option<CompiledSierraWithBlockN>, MadaraStorageError> {
        let Some(res) =
            self.db.get_pinned_cf(&self.get_column(CLASS_COMPILED_COLUMN), &compiled_class_hash.to_bytes_be())?
        else {
            return Ok(None);
        };
        Ok(Some(bincode::deserialize(&res)?))
    }
}
