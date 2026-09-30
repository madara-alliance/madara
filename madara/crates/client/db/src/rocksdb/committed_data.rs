//! Versioned private dataset records. No contract storage or state diff is written here.
use super::*;
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use rocksdb::WriteBatch;

const PREFIX: &[u8] = b"committed_data_snapshot_v1/";
const USAGE_KEY: &[u8] = b"committed_data_usage_v1";

fn key(root: Felt, publisher: Felt) -> Vec<u8> {
    let mut key = PREFIX.to_vec();
    key.extend_from_slice(&root.to_bytes_be());
    key.extend_from_slice(&publisher.to_bytes_be());
    key
}

fn decode(bytes: &[u8]) -> Result<Vec<Felt>> {
    anyhow::ensure!(!bytes.is_empty() && bytes.len() % 32 == 0, "Invalid committed-data record length");
    anyhow::ensure!(bytes.len() / 32 <= MAX_COMMITTED_DATA_VALUES, "Oversized committed-data record");
    bytes
        .chunks_exact(32)
        .map(|chunk| {
            let raw: [u8; 32] = chunk.try_into()?;
            let value = Felt::from_bytes_be(&raw);
            anyhow::ensure!(value.to_bytes_be() == raw, "Noncanonical committed-data value");
            Ok(value)
        })
        .collect()
}

impl RocksDBStorage {
    pub(super) fn committed_data_values(&self, root: Felt, publisher: Felt) -> Result<Option<Vec<Felt>>> {
        let cf = self.inner.get_column(meta::META_COLUMN);
        self.inner.db.get_pinned_cf(&cf, key(root, publisher))?.map(|bytes| decode(&bytes)).transpose()
    }

    // The backend serializes imports, including the quota check and write.
    pub(super) fn store_committed_data_values(
        &self,
        root: Felt,
        publisher: Felt,
        values: &[Felt],
        max_bytes: u64,
    ) -> Result<()> {
        anyhow::ensure!(!values.is_empty() && values.len() <= MAX_COMMITTED_DATA_VALUES, "Invalid dataset length");
        let cf = self.inner.get_column(meta::META_COLUMN);
        let key = key(root, publisher);
        let bytes: Vec<u8> = values.iter().flat_map(Felt::to_bytes_be).collect();
        if let Some(existing) = self.inner.db.get_pinned_cf(&cf, &key)? {
            anyhow::ensure!(&*existing == bytes.as_slice(), "Conflicting immutable committed-data dataset");
            return Ok(());
        }
        let used = self.inner.db.get_pinned_cf(&cf, USAGE_KEY)?;
        let used = used.map(|raw| <[u8; 8]>::try_from(&*raw).map(u64::from_be_bytes)).transpose()?.unwrap_or(0);
        let next = used
            .checked_add(bytes.len() as u64 + key.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("Committed-data quota overflow"))?;
        anyhow::ensure!(
            next <= max_bytes,
            "Committed-data storage quota exceeded; increase configured limit or archive safely"
        );
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, key, bytes);
        batch.put_cf(&cf, USAGE_KEY, next.to_be_bytes());
        let mut options = WriteOptions::default();
        options.set_sync(true);
        options.disable_wal(false);
        self.inner.db.write_opt(batch, &options)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_values_must_be_canonical_and_bounded() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[0_u8; 31]).is_err());
        assert!(decode(&[0xff_u8; 32]).is_err());
        assert_eq!(decode(&Felt::MAX.to_bytes_be()).unwrap(), vec![Felt::MAX]);
    }
}
