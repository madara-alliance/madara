use crate::import::BlockImporter;
use anyhow::Context;
use mc_db::MadaraBackend;
use mp_convert::Felt;
use mp_utils::service::ServiceContext;
use std::sync::Arc;
use std::time::Duration;

// HARDCODED POC VALUES - Change these for different worker nodes
const WORKER_ID: u8 = 0;  // Set to 0, 1, or 2 for different workers
const WORKER_COUNT: u8 = 1;  // Total number of workers
const COORDINATOR_URL: &str = "http://localhost:9942";  // Coordinator's admin RPC port
const ENABLED: bool = false;  // Set to true to enable worker mode

/// Trie Worker Task for distributed trie computation POC
/// 
/// This task runs on worker nodes that:
/// 1. Sync blocks from the coordinator (without computing tries)
/// 2. Compute tries for blocks assigned to them (based on block_number % WORKER_COUNT)
/// 3. Submit computed state roots back to coordinator via admin RPC
/// 
/// Each worker is assigned blocks based on modulo arithmetic:
/// - Worker 0: blocks 0, 3, 6, 9, ...
/// - Worker 1: blocks 1, 4, 7, 10, ...
/// - Worker 2: blocks 2, 5, 8, 11, ...
pub struct TrieWorkerTask {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    http_client: reqwest::Client,
}

impl TrieWorkerTask {
    pub fn new(backend: Arc<MadaraBackend>, importer: Arc<BlockImporter>) -> Self {
        Self {
            backend,
            importer,
            http_client: reqwest::Client::new(),
        }
    }
    
    pub fn is_enabled() -> bool {
        ENABLED
    }
    
    pub async fn run(self, mut ctx: ServiceContext) -> anyhow::Result<()> {
        if !ENABLED {
            tracing::info!("Trie worker is disabled (ENABLED = false)");
            return Ok(());
        }
        
        tracing::info!(
            "🔧 Trie Worker {} starting (total workers: {}, coordinator: {})",
            WORKER_ID,
            WORKER_COUNT,
            COORDINATOR_URL
        );
        
        loop {
            tokio::select! {
                _ = ctx.cancelled() => {
                    tracing::info!("Trie worker shutting down");
                    break;
                }
                result = self.process_tick() => {
                    if let Err(e) = result {
                        tracing::error!("Error in worker tick: {:?}", e);
                    }
                }
            }
            
            // Sleep between processing rounds
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        
        Ok(())
    }
    
    async fn process_tick(&self) -> anyhow::Result<()> {
        tracing::info!(">>>>>>>>Worker {} processing tick", WORKER_ID);
        
        // Find blocks that need trie computation
        let latest_trie_update = self.backend.get_latest_applied_trie_update()?;
        let latest_confirmed = self.backend.latest_confirmed_block_n();
        let latest_block = self.backend.latest_block_n().unwrap_or(0);
        
        // Calculate next block to process
        // If None (no tries computed yet), start from 0
        // If Some(n), next is n+1
        let next_block_to_process = latest_trie_update
            .map(|n| n + 1)
            .unwrap_or(0);
        
        tracing::info!(
            "📊 Worker {} status: latest_trie={:?}, next_to_process={}, latest_confirmed={:?}, latest_block={}",
            WORKER_ID,
            latest_trie_update,
            next_block_to_process,
            latest_confirmed,
            latest_block
        );
        
        // Use latest_block (any saved block) instead of latest_confirmed for worker nodes
        if next_block_to_process > latest_block {
            // No new blocks to process
            tracing::info!(
                "Worker {}: No new blocks to process yet (waiting for block {})",
                WORKER_ID,
                next_block_to_process
            );
            return Ok(());
        }
        
        tracing::info!(
            "🔍 Worker {} checking blocks {} to {} (latest_trie={:?}, latest_block={})",
            WORKER_ID,
            next_block_to_process,
            latest_block,
            latest_trie_update,
            latest_block
        );
        
        // Process blocks that are assigned to this worker
        let mut processed_count = 0;
        let mut assigned_count = 0;
        
        for block_n in next_block_to_process..=latest_block {
            // Check if this block is assigned to this worker
            if block_n % WORKER_COUNT as u64 == WORKER_ID as u64 {
                assigned_count += 1;
                tracing::info!("🎯 Worker {} assigned to process block {}", WORKER_ID, block_n);
                
                match self.process_block(block_n).await {
                    Ok(_) => {
                        processed_count += 1;
                        tracing::info!("✅ Worker {} successfully completed block {}", WORKER_ID, block_n);
                    }
                    Err(e) => {
                        tracing::error!("❌ Worker {} failed block {}: {:#}", WORKER_ID, block_n, e);
                        // Continue with next block instead of failing completely
                    }
                }
            } else {
                tracing::debug!(
                    "⏭️  Worker {} skipping block {} (assigned to worker {})",
                    WORKER_ID,
                    block_n,
                    block_n % WORKER_COUNT as u64
                );
            }
        }
        
        tracing::info!(
            "📊 Worker {} tick complete: processed {}/{} assigned blocks",
            WORKER_ID,
            processed_count,
            assigned_count
        );
        
        Ok(())
    }
    
    async fn process_block(&self, block_n: u64) -> anyhow::Result<()> {
        let start_time = std::time::Instant::now();
        tracing::info!("🔧 Worker {} starting to process block {}", WORKER_ID, block_n);
        
        // Get the last block we computed trie for
        // If None (no tries computed yet), start from block 0
        // If Some(n), start from block n+1
        let start_block = self.backend.get_latest_applied_trie_update()?
            .map(|n| n + 1)
            .unwrap_or(0);
        
        tracing::info!(
            "📦 Worker {} will squash state diffs from block {} to {} ({} blocks)",
            WORKER_ID,
            start_block,
            block_n,
            block_n - start_block + 1
        );
        
        // Collect state diffs from start_block to block_n (inclusive)
        tracing::info!("📥 Worker {} fetching state diffs...", WORKER_ID);
        let fetch_start = std::time::Instant::now();
        let mut state_diffs = Vec::new();
        for bn in start_block..=block_n {
            let block_view = self.backend.block_view_on_confirmed(bn)
                .with_context(|| format!("Block {} not found", bn))?;
            let state_diff = block_view.get_state_diff()
                .with_context(|| format!("Failed to get state diff for block {}", bn))?;
            state_diffs.push(state_diff);
            
            if bn % 10 == 0 || bn == block_n {
                tracing::info!("  📄 Fetched state diff for block {}/{}", bn, block_n);
            }
        }
        tracing::info!(
            "✅ Worker {} fetched {} state diffs in {:?}",
            WORKER_ID,
            state_diffs.len(),
            fetch_start.elapsed()
        );
        
        // Squash state diffs using the snap_sync logic
        tracing::info!("🔄 Worker {} squashing state diffs...", WORKER_ID);
        let squash_start = std::time::Instant::now();
        let raw_squashed = self.squash_state_diffs(state_diffs);
        tracing::info!(
            "  📊 Raw squashed: {} storage diffs, {} deployed, {} declared",
            raw_squashed.storage_diffs.len(),
            raw_squashed.deployed_contracts.len(),
            raw_squashed.declared_classes.len()
        );
        
        let pre_range_block = if start_block == 0 { None } else { Some(start_block - 1) };
        tracing::info!("🗜️  Worker {} compressing state diff (pre_range_block={:?})...", WORKER_ID, pre_range_block);
        let compress_start = std::time::Instant::now();
        let squashed_diff = crate::sync_utils::compress_state_diff(
            raw_squashed,
            pre_range_block,
            self.backend.clone(),
        )
        .await
        .context("Squashing state diffs")?;
        tracing::info!(
            "✅ Worker {} compressed state diff in {:?}",
            WORKER_ID,
            compress_start.elapsed()
        );
        tracing::info!(
            "  📊 Compressed: {} storage diffs, {} deployed, {} declared",
            squashed_diff.storage_diffs.len(),
            squashed_diff.deployed_contracts.len(),
            squashed_diff.declared_classes.len()
        );
        tracing::info!(
            "✅ Worker {} total squashing time: {:?}",
            WORKER_ID,
            squash_start.elapsed()
        );
        
        // Compute trie in rayon pool with squashed diff
        tracing::info!("🌳 Worker {} computing trie for blocks {} to {}...", WORKER_ID, start_block, block_n);
        let trie_start = std::time::Instant::now();
        let backend_clone = self.backend.clone();
        let state_root = self.importer
            .run_in_rayon_pool_global(move |_| {
                tracing::info!("  ⚙️  Computing trie in rayon pool...");
                backend_clone
                    .write_access()
                    .apply_to_global_trie(start_block, vec![squashed_diff].iter())
            })
            .await
            .context("Computing trie")?;
        tracing::info!(
            "✅ Worker {} computed trie in {:?}, state_root=0x{:x}",
            WORKER_ID,
            trie_start.elapsed(),
            state_root
        );
        
        // Submit result to coordinator
        tracing::info!("📤 Worker {} submitting result to coordinator...", WORKER_ID);
        let submit_start = std::time::Instant::now();
        self.submit_result(block_n, state_root).await?;
        tracing::info!(
            "✅ Worker {} submitted result in {:?}",
            WORKER_ID,
            submit_start.elapsed()
        );
        
        // Update local marker to track what we've processed
        // This prevents reprocessing the same block on the next tick
        self.backend.write_latest_applied_trie_update(&Some(block_n))?;
        tracing::debug!("📝 Worker {} updated local trie marker to {}", WORKER_ID, block_n);
        
        tracing::info!(
            "🎉 Worker {} completed block {} in {:?}",
            WORKER_ID,
            block_n,
            start_time.elapsed()
        );
        
        Ok(())
    }
    
    /// Squash multiple state diffs into one (simple merge without compression)
    /// This is similar to StateDiffMap::apply_state_diff but for a batch
    fn squash_state_diffs(&self, state_diffs: Vec<mp_state_update::StateDiff>) -> mp_state_update::StateDiff {
        use std::collections::HashMap;
        use mp_state_update::*;
        
        let mut storage_map: HashMap<Felt, HashMap<Felt, Felt>> = HashMap::new();
        let mut deployed_contracts: HashMap<Felt, Felt> = HashMap::new();
        let mut declared_classes: HashMap<Felt, Felt> = HashMap::new();
        let mut old_declared_contracts: std::collections::HashSet<Felt> = std::collections::HashSet::new();
        let mut nonces: HashMap<Felt, Felt> = HashMap::new();
        let mut replaced_classes: HashMap<Felt, Felt> = HashMap::new();
        
        // Merge all state diffs
        for state_diff in state_diffs {
            // Storage diffs
            for contract_diff in state_diff.storage_diffs {
                let contract_storage = storage_map.entry(contract_diff.address).or_default();
                for entry in contract_diff.storage_entries {
                    contract_storage.insert(entry.key, entry.value);
                }
            }
            
            // Deployed contracts
            for item in state_diff.deployed_contracts {
                deployed_contracts.insert(item.address, item.class_hash);
            }
            
            // Declared classes
            for item in state_diff.declared_classes {
                declared_classes.insert(item.class_hash, item.compiled_class_hash);
            }
            
            // Nonces
            for item in state_diff.nonces {
                nonces.insert(item.contract_address, item.nonce);
            }
            
            // Replaced classes
            for item in state_diff.replaced_classes {
                replaced_classes.insert(item.contract_address, item.class_hash);
            }
            
            // Old declared contracts
            for class_hash in state_diff.old_declared_contracts {
                old_declared_contracts.insert(class_hash);
            }
        }
        
        // Convert back to StateDiff format
        let storage_diffs: Vec<ContractStorageDiffItem> = storage_map
            .into_iter()
            .map(|(address, storage_entries)| {
                let storage_entries: Vec<StorageEntry> = storage_entries
                    .into_iter()
                    .map(|(key, value)| StorageEntry { key, value })
                    .collect();
                ContractStorageDiffItem { address, storage_entries }
            })
            .collect();
        
        let deployed_contracts: Vec<DeployedContractItem> = deployed_contracts
            .into_iter()
            .map(|(address, class_hash)| DeployedContractItem { address, class_hash })
            .collect();
        
        let declared_classes: Vec<DeclaredClassItem> = declared_classes
            .into_iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem { class_hash, compiled_class_hash })
            .collect();
        
        let nonces: Vec<NonceUpdate> = nonces
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate { contract_address, nonce })
            .collect();
        
        let replaced_classes: Vec<ReplacedClassItem> = replaced_classes
            .into_iter()
            .map(|(contract_address, class_hash)| ReplacedClassItem { contract_address, class_hash })
            .collect();
        
        let old_declared_contracts: Vec<Felt> = old_declared_contracts.into_iter().collect();
        
        StateDiff {
            storage_diffs,
            deployed_contracts,
            declared_classes,
            old_declared_contracts,
            nonces,
            replaced_classes,
        }
    }
    
    async fn submit_result(&self, block_n: u64, state_root: Felt) -> anyhow::Result<()> {
        let state_root_hex = format!("0x{:x}", state_root);
        
        tracing::debug!("Submitting result for block {} to {}", block_n, COORDINATOR_URL);
        
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "madara_submitTrieResult",
            "params": [block_n, state_root_hex],
            "id": 1
        });
        
        let response = self.http_client
            .post(COORDINATOR_URL)
            .json(&payload)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("Sending result to coordinator")?;
        
        if response.status().is_success() {
            let body = response.text().await?;
            tracing::info!("✅ Submitted result for block {} to coordinator: {}", block_n, body);
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Failed to submit result (status {}): {}", status, body);
        }
    }
}

