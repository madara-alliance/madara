//! Resumable-by-presence replication of immutable datasets. No partial dataset is advertised.
use anyhow::Context;
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use mc_db::{MadaraBackend, MadaraStorageRead, MadaraStorageWrite};
use mc_gateway_client::GatewayProvider;
use mp_convert::Felt;
use mp_gateway::committed_data::{CommittedDataPage, ROOTS_PER_PAGE, VALUES_PER_PAGE};
use std::{sync::Arc, time::Duration};

/// Checks metadata/order before extending the bounded assembly buffer.
fn append_page(
    root: Felt,
    count: Option<u32>,
    values: &mut Vec<(Felt, Felt)>,
    page: CommittedDataPage,
) -> anyhow::Result<u32> {
    anyhow::ensure!(page.root == root && page.start as usize == values.len(), "Committed-data page identity mismatch");
    anyhow::ensure!((1..=MAX_COMMITTED_DATA_VALUES).contains(&(page.count as usize)), "Invalid committed-data count");
    anyhow::ensure!(count.is_none_or(|count| count == page.count), "Committed-data count changed during transfer");
    let remaining = (page.count as usize).checked_sub(values.len()).context("Committed-data offset exceeds count")?;
    anyhow::ensure!(
        remaining > 0 && page.entries.len() == remaining.min(VALUES_PER_PAGE),
        "Invalid committed-data page length"
    );
    values.extend(page.entries);
    Ok(page.count)
}

/// Repeats discovery from the start on every pass: lexical cursors can miss concurrent lower roots.
/// Imported datasets are skipped after restart; interrupted downloads are retried in full. The
/// upstream gateway is a configured availability source, not an authority for Oracle publication.
pub(crate) async fn sync_available(backend: &Arc<MadaraBackend>, client: &GatewayProvider) -> anyhow::Result<()> {
    let mut after = None;
    loop {
        let roots = tokio::time::timeout(Duration::from_secs(30), client.get_committed_data_roots(after)).await??;
        anyhow::ensure!(roots.len() <= ROOTS_PER_PAGE, "Oversized committed-data root page");
        let mut previous = after;
        for root in &roots {
            anyhow::ensure!(previous.is_none_or(|previous| *root > previous), "Unordered committed-data root page");
            previous = Some(*root);
        }
        for root in &roots {
            let root = *root;
            let reader = Arc::clone(backend);
            // Presence is durable because metadata and all pages were written atomically.
            if tokio::task::spawn_blocking(move || reader.db.get_committed_data_count(root)).await??.is_some() {
                continue;
            }
            let mut values = Vec::new();
            let mut count = None;
            loop {
                let page = tokio::time::timeout(
                    Duration::from_secs(30),
                    client.get_committed_data_page(root, values.len() as u32),
                )
                .await??
                .context("Advertised committed dataset is unavailable")?;
                let total = append_page(root, count, &mut values, page)?;
                count = Some(total);
                if values.len() == total as usize {
                    break;
                }
            }
            // Authenticate the dataset's Patricia commitment and persist with WAL+fsync.
            backend.import_committed_data_snapshot(root, values).await?;
            tracing::info!(root = %root, "Replicated committed-data dataset");
        }
        if roots.len() < ROOTS_PER_PAGE {
            break;
        }
        after = roots.last().copied();
    }
    // Labels are copied only after their authenticated datasets exist locally. A concurrently
    // imported version may not have appeared in the root pass; retry the next complete scan.
    let mut after = None;
    loop {
        let versions =
            tokio::time::timeout(Duration::from_secs(30), client.get_committed_data_versions(after)).await??;
        anyhow::ensure!(versions.len() <= ROOTS_PER_PAGE, "Oversized committed-data version page");
        for version in &versions {
            anyhow::ensure!(after.is_none_or(|previous| *version > previous), "Unordered committed-data version page");
            after = Some(*version);
        }
        let reader = Arc::clone(backend);
        let last = versions.len() < ROOTS_PER_PAGE;
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            for version in versions {
                if !reader.db.has_committed_data_version(version)? {
                    reader.db.write_committed_data_version(version, false)?;
                }
            }
            Ok(())
        })
        .await??;
        if last {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn committed_data_http_replication_authenticates_pages_and_survives_source_loss_and_reopen() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        use mc_db::{rocksdb::RocksDBConfig, MadaraBackendConfig};
        use mp_chain_config::ChainConfig;
        let mut values: Vec<_> = (0..4097_u32).map(|i| (Felt::from(i), Felt::from(i))).collect();
        // Opaque felts, including encoded symbols and high-bit values, survive replication.
        values[0].0 = Felt::from_hex_unchecked("0x4254432d5553442d505542"); // BTC-USD-PUB
        values[4096].0 = Felt::from_hex_unchecked("0x700000000000000000000000000000000000000000000000000000000000001");
        values.sort_unstable_by_key(|entry| entry.0);
        let root = CommittedDataSet::new(values.clone()).unwrap().root();
        let source = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let namespace = mp_gateway::committed_data::CommittedDataUpdate {
            application_id: Felt::ONE,
            dataset_id: Felt::TWO,
            base_root: None,
        };
        source.update_committed_data(namespace, root, values.clone()).await.unwrap();
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/feeder_gateway/get_committed_data_roots");
                then.json_body_obj(&vec![root]);
            })
            .await;
        let versions = source.db.list_committed_data_versions(None).unwrap();
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/feeder_gateway/get_committed_data_versions");
                then.json_body_obj(&versions);
            })
            .await;
        let mut pages = Vec::new();
        for start in [0, 4096] {
            let (count, values) = source.db.get_committed_data_page(root, start).unwrap().unwrap();
            pages.push(
                server
                    .mock_async(|when, then| {
                        when.method(httpmock::Method::GET)
                            .path("/feeder_gateway/get_committed_data")
                            .query_param("root", format!("{root:#x}"))
                            .query_param("start", start.to_string());
                        then.json_body_obj(&Some(CommittedDataPage { root, count, start, entries: values }));
                    })
                    .await,
            );
        }
        let client = GatewayProvider::new(
            server.url("/gateway/").parse().unwrap(),
            server.url("/feeder_gateway/").parse().unwrap(),
        );
        let directory = tempfile::tempdir().unwrap();
        let open = || {
            MadaraBackend::open_rocksdb(
                directory.path(),
                Arc::new(ChainConfig::madara_test()),
                MadaraBackendConfig::default(),
                RocksDBConfig::default(),
                Default::default(),
            )
            .unwrap()
        };
        {
            let replica = open();
            sync_available(&replica, &client).await.unwrap();
            sync_available(&replica, &client).await.unwrap();
            for page in &pages {
                page.assert_hits_async(1).await;
            }
            assert_eq!(replica.committed_data_witness(root, values[0].0).unwrap().unwrap().value, values[0].1);
            assert_eq!(replica.committed_data_witness(root, values[4096].0).unwrap().unwrap().value, values[4096].1);
        }
        for page in &pages {
            page.delete_async().await;
        } // Source no longer serves datasets.
        drop(source);
        let replica = open();
        assert_eq!(replica.committed_data_witness(root, values[0].0).unwrap().unwrap().value, values[0].1);
        assert!(replica.committed_data_witness(root, values[4096].0).unwrap().unwrap().verify());
        assert_eq!(replica.db.list_committed_data_roots(None).unwrap(), vec![root]);
        assert_eq!(replica.db.list_committed_data_versions(None).unwrap(), versions);
        // The source is gone, but the promoted replica can merge into its replicated namespace.
        let key = values[0].0;
        let old = values[4096];
        values[0].1 = Felt::MAX;
        let next = CommittedDataSet::new(values).unwrap().root();
        replica
            .update_committed_data(
                mp_gateway::committed_data::CommittedDataUpdate { base_root: Some(root), ..namespace },
                next,
                vec![(key, Felt::MAX)],
            )
            .await
            .unwrap();
        assert_eq!(replica.committed_data_witness(next, old.0).unwrap().unwrap().value, old.1);
        assert!(replica.committed_data_witness(root, key).unwrap().unwrap().verify());
    }

    #[tokio::test]
    async fn committed_data_http_corruption_is_never_persisted() {
        use blockifier::execution::syscalls::committed_data::CommittedDataSet;
        let root = CommittedDataSet::new(vec![(Felt::ZERO, Felt::ONE)]).unwrap().root();
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.path("/feeder_gateway/get_committed_data_roots");
                then.json_body_obj(&vec![root]);
            })
            .await;
        server
            .mock_async(|when, then| {
                when.path("/feeder_gateway/get_committed_data");
                then.json_body_obj(&Some(CommittedDataPage {
                    root,
                    start: 0,
                    count: 1,
                    entries: vec![(Felt::ZERO, Felt::TWO)],
                }));
            })
            .await;
        let client = GatewayProvider::new(
            server.url("/gateway/").parse().unwrap(),
            server.url("/feeder_gateway/").parse().unwrap(),
        );
        let replica = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
        assert!(sync_available(&replica, &client).await.is_err());
        assert!(replica.db.list_committed_data_roots(None).unwrap().is_empty());
    }

    #[test]
    fn committed_data_transfer_rejects_wrong_order_count_and_short_pages() {
        let root = Felt::ONE;
        let page = CommittedDataPage { root, start: 0, count: 4097, entries: vec![(Felt::ZERO, Felt::TWO); 4096] };
        let mut values = Vec::new();
        assert_eq!(append_page(root, None, &mut values, page.clone()).unwrap(), 4097);
        assert!(append_page(root, Some(4097), &mut values, page).is_err());
        for bad in [
            CommittedDataPage { root: Felt::TWO, start: 4096, count: 4097, entries: vec![(Felt::ZERO, Felt::ONE)] },
            CommittedDataPage { root, start: 4096, count: 4098, entries: vec![(Felt::ZERO, Felt::ONE); 2] },
            CommittedDataPage { root, start: 4096, count: 4097, entries: vec![] },
        ] {
            assert!(append_page(root, Some(4097), &mut values, bad).is_err());
        }
        append_page(
            root,
            Some(4097),
            &mut values,
            CommittedDataPage { root, start: 4096, count: 4097, entries: vec![(Felt::ZERO, Felt::ONE)] },
        )
        .unwrap();
        assert_eq!(values.len(), 4097);
    }
}
