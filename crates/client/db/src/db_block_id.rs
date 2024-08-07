use core::fmt;

use dp_block::BlockId;

use crate::{DeoxysBackend, DeoxysStorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbBlockId {
    Pending,
    BlockN(u64),
}

impl DbBlockId {
    pub fn is_pending(&self) -> bool {
        matches!(self, DbBlockId::Pending)
    }
}

pub trait DbBlockIdResolvable {
    fn resolve_db_block_id(&self, backend: &DeoxysBackend) -> Result<Option<DbBlockId>, DeoxysStorageError>;
}

impl DbBlockIdResolvable for BlockId {
    fn resolve_db_block_id(&self, backend: &DeoxysBackend) -> Result<Option<DbBlockId>, DeoxysStorageError> {
        backend.id_to_storage_type(self)
    }
}

impl DbBlockIdResolvable for starknet_core::types::BlockId {
    fn resolve_db_block_id(&self, backend: &DeoxysBackend) -> Result<Option<DbBlockId>, DeoxysStorageError> {
        backend.id_to_storage_type(&(*self).into())
    }
}

impl DbBlockIdResolvable for DbBlockId {
    fn resolve_db_block_id(&self, _backend: &DeoxysBackend) -> Result<Option<DbBlockId>, DeoxysStorageError> {
        Ok(Some(*self))
    }
}

impl fmt::Display for DbBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "#<pending>"),
            Self::BlockN(block_n) => write!(f, "#{block_n}"),
        }
    }
}

impl DeoxysBackend {
    pub fn resolve_block_id(&self, id: &impl DbBlockIdResolvable) -> Result<Option<DbBlockId>, DeoxysStorageError> {
        id.resolve_db_block_id(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_db_block_id() {
        assert!(DbBlockId::Pending.is_pending());
        assert!(!DbBlockId::BlockN(0).is_pending());
    }
}
