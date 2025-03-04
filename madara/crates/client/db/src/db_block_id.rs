use crate::{MadaraBackend, MadaraStorageError};
use core::fmt;
use mp_block::{BlockId, BlockTag};

/// Raw DB Block Id. Queries using these block ids will be able to index past the latest current block.
/// The sync process has not yet marked these blocks as fully imported. You should use [`DbBlockId`] instead,
/// as queries using it will not return anything past the current latest block.
/// The sync process may use [`RawDbBlockId`]s to get the blocks it has saved past the latest full block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RawDbBlockId {
    Pending,
    Number(u64),
}

impl RawDbBlockId {
    pub fn from_block_n(block_n: Option<u64>) -> Self {
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
        matches!(self, Self::Pending)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DbBlockId {
    Pending,
    Number(u64),
}

impl DbBlockId {
    pub fn from_block_n(block_n: Option<u64>) -> Self {
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
        matches!(self, Self::Pending)
    }
}

pub trait DbBlockIdResolvable {
    fn resolve_db_block_id(&self, backend: &MadaraBackend) -> Result<Option<RawDbBlockId>, MadaraStorageError>;
}

impl DbBlockIdResolvable for BlockId {
    fn resolve_db_block_id(&self, backend: &MadaraBackend) -> Result<Option<RawDbBlockId>, MadaraStorageError> {
        let block_id = match self {
            BlockId::Hash(hash) => backend.block_hash_to_block_n(hash)?.map(DbBlockId::Number),
            BlockId::Number(block_n) => Some(DbBlockId::Number(*block_n)),
            BlockId::Tag(BlockTag::Latest) => backend.head_status().full_block.get().map(DbBlockId::Number),
            BlockId::Tag(BlockTag::Pending) => Some(DbBlockId::Pending),
        };
        let Some(block_id) = block_id else {
            return Ok(None);
        };
        block_id.resolve_db_block_id(backend)
    }
}

impl DbBlockIdResolvable for DbBlockId {
    fn resolve_db_block_id(&self, backend: &MadaraBackend) -> Result<Option<RawDbBlockId>, MadaraStorageError> {
        match self {
            DbBlockId::Pending => Ok(Some(RawDbBlockId::Pending)),
            DbBlockId::Number(block_n) => {
                let Some(latest_block) = backend.head_status().full_block.get() else {
                    // No blocks on DB.
                    return Ok(None);
                };
                if block_n > &latest_block {
                    // Cannot index past the latest block.
                    return Ok(None);
                }
                Ok(Some(RawDbBlockId::Number(*block_n)))
            }
        }
    }
}

impl DbBlockIdResolvable for RawDbBlockId {
    fn resolve_db_block_id(&self, _backend: &MadaraBackend) -> Result<Option<RawDbBlockId>, MadaraStorageError> {
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

impl fmt::Display for RawDbBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "#<pending>"),
            Self::Number(block_n) => write!(f, "#{block_n}"),
        }
    }
}

impl MadaraBackend {
    pub fn resolve_block_id(&self, id: &impl DbBlockIdResolvable) -> Result<Option<RawDbBlockId>, MadaraStorageError> {
        id.resolve_db_block_id(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_block_id() {
        assert!(DbBlockId::Pending.is_pending());
        assert!(!DbBlockId::Number(0).is_pending());
    }
}
