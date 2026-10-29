//! Total local loss recovery: chain-only peer supplies canonical blocks, while Mongo alone
//! supplies committed-data values. No database snapshot or test-only storage injection is used.
use super::{comparable_trace, rpc, save, CommittedDataSet, Node};
use anyhow::{ensure, Context, Result};
use cairo_vm::types::layout_name::LayoutName;
use futures::TryStreamExt;
use generate_pie::{generate_pie, ChainConfig, OsHintsConfiguration, PieGenerationInput};
use mongodb::bson::{doc, Document};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

// Feeder protocol page width; assert the full final page as well as every preceding page.
const VALUES_PER_PAGE: usize = 4096;

/// Wait for the ordinary exporter to publish complete manifests before destroying the source.
/// The fixture owns a unique Mongo database, so unrelated backups cannot satisfy this barrier.
async fn exported_manifests(source: &Node, datasets: &[&CommittedDataSet]) -> Result<Vec<String>> {
    let mongo = mongodb::Client::with_uri_str("mongodb://127.0.0.1:27017").await?;
    let collection =
        mongo.database(source.backup_database()).collection::<Document>("mempool_transactions_committed_manifests_v1");
    for _ in 0..120 {
        let documents: Vec<Document> = collection.find(doc! {}).await?.try_collect().await?;
        let mut ids = Vec::new();
        for dataset in datasets {
            for document in &documents {
                let manifest: Value = serde_json::from_slice(document.get_binary_generic("payload")?)?;
                if manifest["root"] == json!(dataset.root()) {
                    ids.push(document.get_str("_id")?.to_owned());
                    break;
                }
            }
        }
        if ids.len() == datasets.len() {
            return Ok(ids);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    anyhow::bail!("source exporter did not back up every dataset")
}

/// Compare all values, boundary witnesses and complete traces/receipts with independent
/// pre-loss references. Checking only roots or the first page could hide an incomplete restore.
async fn verify_history(node: &Node, datasets: &[&CommittedDataSet], observations: &[Value]) -> Result<()> {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?;
    for dataset in datasets {
        for (page_index, expected) in dataset.entries().chunks(VALUES_PER_PAGE).enumerate() {
            let start = page_index * VALUES_PER_PAGE;
            let page: Value = client
                .get(format!("{}get_committed_data?root={:#x}&start={start}", node.feeder, dataset.root()))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            ensure!(
                page["root"] == json!(dataset.root())
                    && page["count"] == json!(dataset.entries().len())
                    && page["start"] == json!(start)
                    && page["entries"] == json!(expected),
                "restored page differs at offset {start}"
            );
        }
        for index in [0, dataset.entries().len() / 2, dataset.entries().len() - 1] {
            let key = dataset.entries()[index].0;
            let witness: Value = client
                .get(format!("{}get_committed_data_witness?root={:#x}&key={key:#x}", node.feeder, dataset.root()))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            ensure!(
                witness == json!(dataset.witness(key).context("reference key absent")?),
                "restored Merkle path differs for {key:#x}"
            );
        }
    }
    for observation in observations {
        let hash = &observation["receipt"]["transaction_hash"];
        ensure!(
            rpc(&node.rpc, "starknet_getTransactionReceipt", json!([hash])).await? == observation["receipt"],
            "restored historical receipt differs"
        );
        ensure!(
            comparable_trace(rpc(&node.rpc, "starknet_traceTransaction", json!([hash])).await?)
                == comparable_trace(observation["trace"].clone()),
            "restored historical trace differs"
        );
    }
    Ok(())
}

async fn snos_replay(node: &Node, blocks: Vec<u64>) -> Result<Value> {
    let replay = generate_pie(PieGenerationInput {
        rpc_url: node.rpc.clone(),
        committed_data_rpc_url: Some(node.feeder.clone()),
        blocks: blocks.clone(),
        chain_config: ChainConfig::default_with_chain("MADARA_DEVNET"),
        os_hints_config: OsHintsConfiguration { use_committed_data: true, use_kzg_da: false, ..Default::default() },
        output_path: None,
        layout: LayoutName::all_cairo,
        versioned_constants: None,
        public_keys: None,
    })
    .await
    .context("historical SNOS replay")?;
    replay.output.cairo_pie.run_validity_checks()?;
    Ok(json!({"blocks":blocks,"os_resources":replay.output.cairo_pie.execution_resources,
        "os_transactions":replay.output.txs_trace}))
}

pub(super) async fn total_database_loss(
    source: &mut Node,
    chain_peer: &Node,
    datasets: &[&CommittedDataSet],
    observations: &[Value],
    target: u64,
) -> Result<()> {
    let manifests = exported_manifests(source, datasets).await?;
    let expected_head = rpc(&source.rpc, "starknet_getBlockWithTxHashes", json!([{"block_number":target}])).await?;
    // The source has already restarted: replay must use historical canonical trie logs,
    // not a newer snapshot. This also separates a pre-existing replay failure from recovery.
    save("pre_loss_multi_block_control", &snos_replay(source, vec![target - 1, target]).await?)?;
    let started = Instant::now();
    source.recover_total_loss(chain_peer, target, &manifests).await?;
    verify_history(source, datasets, observations).await?;
    for block in [target - 1, target] {
        save(&format!("mongo_recovered_single_{block}"), &snos_replay(source, vec![block]).await?)?;
    }
    save("mongo_recovered_snos", &snos_replay(source, vec![target - 1, target]).await?)?;
    let recovered_ms = started.elapsed().as_millis();
    let mut fresh_replica = Node::start(Some(source)).await?;
    fresh_replica.wait_for_block(target).await?;
    fresh_replica.wait_for_datasets(&datasets.iter().map(|tree| tree.root()).collect::<Vec<_>>()).await?;
    verify_history(&fresh_replica, datasets, observations).await?;
    for node in [&mut *source, &mut fresh_replica] {
        node.restart(false).await?;
        verify_history(node, datasets, observations).await?;
        ensure!(
            rpc(&node.rpc, "starknet_getBlockWithTxHashes", json!([{"block_number":target}])).await? == expected_head,
            "recovered canonical block/hash/state root differs"
        );
    }
    save("mongo_recovered_fullnode_snos", &snos_replay(&fresh_replica, vec![target - 1, target]).await?)?;
    rpc(&source.admin, "madara_closeBlock", json!([])).await?;
    let new_height = rpc(&source.rpc, "starknet_blockNumber", json!([])).await?.as_u64().context("block height")?;
    ensure!(new_height > target, "recovered sequencer did not advance");
    fresh_replica.wait_for_block(new_height).await?;
    let new_block = rpc(&source.rpc, "starknet_getBlockWithTxHashes", json!([{"block_number":new_height}])).await?;
    ensure!(
        rpc(&fresh_replica.rpc, "starknet_getBlockWithTxHashes", json!([{"block_number":new_height}])).await?
            == new_block,
        "new full node diverged on post-recovery production"
    );
    save(
        "mongo_total_database_loss",
        &json!({
            "entire_source_directory_deleted":true, "canonical_resync_from_dataset_free_peer":true,
            "datasets_restored_only_from_mongo":true, "manifest_ids":manifests,
            "all_values_checked_per_node":datasets.iter().map(|tree| tree.entries().len()).sum::<usize>(),
            "historical_transactions_checked_per_node":observations.len(), "fresh_fullnode_sync":true,
            "post_recovery_restart":true, "partial_restore_failure_then_retry":true, "canonical_block":expected_head,
            "recovered_multi_block_snos_pie_valid":true,
            "post_recovery_production_block":new_block,
            "source_recovery_and_verification_ms":recovered_ms,
            "source":source.footprint()?, "fresh_fullnode":fresh_replica.footprint()?
        }),
    )?;
    Ok(())
}
