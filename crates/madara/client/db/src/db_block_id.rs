use core::fmt;

use mp_block::BlockId;

use crate::{MadaraBackend, MadaraStorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DbBlockId {
    Pending,
    Number(u64),
}

impl DbBlockId {
    pub fn from_block_n(block_n: Option<u64>) -> DbBlockId {
        match block_n {
            None => Self::Pending,
            Some(block_n) => Self::Number(block_n),
        }
    }

    pub fn block_n(&self) -> Option<u64> {
        match self {
            Self::Pending => None,
            Self::Number(block_n) => Some(*block_n),
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, DbBlockId::Pending)
    }
}

pub trait DbBlockIdResolvable {
    fn resolve_db_block_id(&self, backend: &MadaraBackend) -> Result<Option<DbBlockId>, MadaraStorageError>;
}

impl DbBlockIdResolvable for BlockId {
    fn resolve_db_block_id(&self, backend: &MadaraBackend) -> Result<Option<DbBlockId>, MadaraStorageError> {
        backend.id_to_storage_type(self)
    }
}

impl DbBlockIdResolvable for DbBlockId {
    fn resolve_db_block_id(&self, _backend: &MadaraBackend) -> Result<Option<DbBlockId>, MadaraStorageError> {
        Ok(Some(*self))
    }
}

impl fmt::Display for DbBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "#<pending>"),
            Self::Number(block_n) => write!(f, "#{block_n}"),
        }
    }
}

impl MadaraBackend {
    pub fn resolve_block_id(&self, id: &impl DbBlockIdResolvable) -> Result<Option<DbBlockId>, MadaraStorageError> {
        id.resolve_db_block_id(self)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_db_block_id() {
        assert!(DbBlockId::Pending.is_pending());
        assert!(!DbBlockId::Number(0).is_pending());
    }
}
