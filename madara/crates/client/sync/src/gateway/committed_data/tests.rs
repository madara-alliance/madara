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
    let client =
        GatewayProvider::new(server.url("/gateway/").parse().unwrap(), server.url("/feeder_gateway/").parse().unwrap());
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
    let client =
        GatewayProvider::new(server.url("/gateway/").parse().unwrap(), server.url("/feeder_gateway/").parse().unwrap());
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

#[test]
fn catalog_pages_are_bounded_strictly_ordered_and_advance_the_cursor() {
    validate_catalog_page::<u32>(&[], None).unwrap();
    validate_catalog_page::<u32>(&[], Some(&10)).unwrap();
    validate_catalog_page(&[11, 12], Some(&10)).unwrap();
    let full: Vec<_> = (0..ROOTS_PER_PAGE).collect();
    validate_catalog_page(&full, None).unwrap();
    for page in [vec![1, 1], vec![2, 1], (0..=ROOTS_PER_PAGE).collect()] {
        assert!(validate_catalog_page(&page, None).is_err());
    }
    for page in [[10, 11], [9, 11]] {
        assert!(validate_catalog_page(&page, Some(&10)).is_err());
    }
}

#[tokio::test]
async fn missing_dataset_does_not_publish_a_root_or_namespace() {
    let server = httpmock::MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.path("/feeder_gateway/get_committed_data_roots");
            then.json_body_obj(&vec![Felt::ONE]);
        })
        .await;
    server
        .mock_async(|when, then| {
            when.path("/feeder_gateway/get_committed_data");
            then.json_body_obj(&None::<CommittedDataPage>);
        })
        .await;
    let versions = server
        .mock_async(|when, then| {
            when.path("/feeder_gateway/get_committed_data_versions");
            then.json_body_obj(&Vec::<mp_gateway::committed_data::CommittedDataVersion>::new());
        })
        .await;
    let client =
        GatewayProvider::new(server.url("/gateway/").parse().unwrap(), server.url("/feeder_gateway/").parse().unwrap());
    let replica = MadaraBackend::open_for_testing(Arc::new(mp_chain_config::ChainConfig::madara_test()));
    assert!(sync_available(&replica, &client).await.unwrap_err().to_string().contains("unavailable"));
    assert!(replica.db.list_committed_data_roots(None).unwrap().is_empty());
    assert!(replica.db.list_committed_data_versions(None).unwrap().is_empty());
    versions.assert_hits_async(0).await;
}
