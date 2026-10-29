//! Resumable-by-presence replication of immutable datasets. No partial dataset is advertised.
#[cfg(test)]
mod tests;

use anyhow::Context;
use blockifier::execution::syscalls::committed_data::MAX_COMMITTED_DATA_VALUES;
use mc_db::{MadaraBackend, MadaraStorageRead, MadaraStorageWrite};
use mc_gateway_client::GatewayProvider;
use mp_convert::Felt;
use mp_gateway::committed_data::{CommittedDataPage, ROOTS_PER_PAGE, VALUES_PER_PAGE};
use mp_utils::service::ServiceContext;
use std::{sync::Arc, time::Duration};

const RESCAN_INTERVAL: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Run alongside chain sync. Only downloads and idle waits are cancellation-safe;
/// once a storage operation is admitted, await its completion before acknowledging stop.
pub(super) async fn run(backend: &Arc<MadaraBackend>, client: &GatewayProvider, mut ctx: ServiceContext) {
    while !ctx.is_cancelled() {
        if let Err(error) = sync_available(backend, client, &mut ctx).await {
            if ctx.is_cancelled() {
                return;
            }
            tracing::warn!("Committed-data replication failed; will retry independently of chain sync: {error:#}");
        }
        if ctx.run_until_cancelled(tokio::time::sleep(RESCAN_INTERVAL)).await.is_none() {
            return;
        }
    }
}

/// Drop an unfinished HTTP request on stop, but never wrap a database write in this helper.
async fn download<T>(ctx: &mut ServiceContext, request: impl std::future::Future<Output = T>) -> anyhow::Result<T> {
    anyhow::ensure!(!ctx.is_cancelled(), "Committed-data replication stopped");
    ctx.run_until_cancelled(tokio::time::timeout(REQUEST_TIMEOUT, request))
        .await
        .context("Committed-data replication stopped")?
        .context("Committed-data download timed out")
}

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
/// upstream gateway is a configured availability source, not an authority for application root publication.
pub(crate) async fn sync_available(
    backend: &Arc<MadaraBackend>,
    client: &GatewayProvider,
    ctx: &mut ServiceContext,
) -> anyhow::Result<()> {
    sync_datasets(backend, client, ctx).await?;
    sync_versions(backend, client, ctx).await
}

fn validate_catalog_page<T: Ord>(page: &[T], after: Option<&T>) -> anyhow::Result<()> {
    anyhow::ensure!(page.len() <= ROOTS_PER_PAGE, "Oversized committed-data catalog page");
    anyhow::ensure!(
        page.windows(2).all(|pair| pair[0] < pair[1])
            && after.is_none_or(|after| page.first().is_none_or(|first| first > after)),
        "Unordered committed-data catalog page"
    );
    Ok(())
}

async fn sync_datasets(
    backend: &Arc<MadaraBackend>,
    client: &GatewayProvider,
    ctx: &mut ServiceContext,
) -> anyhow::Result<()> {
    let mut after = None;
    let mut first_error = None;
    loop {
        let roots = download(ctx, client.get_committed_data_roots(after)).await??;
        validate_catalog_page(&roots, after.as_ref())?;
        for root in &roots {
            anyhow::ensure!(!ctx.is_cancelled(), "Committed-data replication stopped");
            if let Err(error) = sync_dataset(backend, client, *root, ctx).await {
                // A missing historical root must not starve later available roots. The next
                // pass retries it; incomplete datasets still never become locally visible.
                tracing::warn!(%root, "Committed-data dataset replication failed: {error:#}");
                first_error.get_or_insert(error);
            }
        }
        if roots.len() < ROOTS_PER_PAGE {
            break;
        }
        after = roots.last().copied();
    }
    first_error.map_or(Ok(()), Err)
}

async fn sync_dataset(
    backend: &Arc<MadaraBackend>,
    client: &GatewayProvider,
    root: Felt,
    ctx: &mut ServiceContext,
) -> anyhow::Result<()> {
    let reader = Arc::clone(backend);
    // Presence is durable because metadata and nodes were written atomically.
    if tokio::task::spawn_blocking(move || reader.db.get_committed_data_count(root)).await??.is_some() {
        return Ok(());
    }
    let mut values = Vec::new();
    let mut count = None;
    loop {
        let page = download(ctx, client.get_committed_data_page(root, values.len() as u32))
            .await??
            .context("Advertised committed dataset is unavailable")?;
        let total = append_page(root, count, &mut values, page)?;
        count = Some(total);
        if values.len() == total as usize {
            break;
        }
    }
    // Authenticate the dataset's Patricia commitment and persist with WAL+fsync.
    anyhow::ensure!(!ctx.is_cancelled(), "Committed-data replication stopped");
    backend.import_committed_data_snapshot(root, values).await?;
    backend.request_committed_data_cleanup();
    tracing::info!(root = %root, "Replicated committed-data dataset");
    Ok(())
}

async fn sync_versions(
    backend: &Arc<MadaraBackend>,
    client: &GatewayProvider,
    ctx: &mut ServiceContext,
) -> anyhow::Result<()> {
    // Labels are copied only after their authenticated datasets exist locally. A concurrently
    // imported version may not have appeared in the root pass; retry the next complete scan.
    let mut after = None;
    loop {
        let versions = download(ctx, client.get_committed_data_versions(after)).await??;
        validate_catalog_page(&versions, after.as_ref())?;
        after = versions.last().copied();
        anyhow::ensure!(!ctx.is_cancelled(), "Committed-data replication stopped");
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
