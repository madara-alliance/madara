use super::*;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct TrieLogHeads {
    pub(super) contract: Option<u64>,
    pub(super) contract_storage: Option<u64>,
    pub(super) class: Option<u64>,
}

impl TrieLogHeads {
    /// Returns the newest revision materialized by any of the three global tries.
    /// Recovery uses it to detect durable trie state even when the tries are temporarily uneven.
    pub(super) fn highest(self) -> Option<u64> {
        [self.contract, self.contract_storage, self.class].into_iter().flatten().max()
    }

    /// Returns the oldest materialized trie revision available as a common recovery ceiling.
    fn lowest(self) -> Option<u64> {
        [self.contract, self.contract_storage, self.class].into_iter().flatten().min()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrieRevertAction {
    Revert { current: u64, target: u64 },
    AlreadyAtTarget(u64),
    OlderThanTarget { current: u64, target: u64 },
    Missing,
}

/// Classifies one trie's relation to the requested revert target.
/// The caller can then keep mutation and no-op logging explicit for every case.
fn trie_revert_action(latest_log_block_n: Option<u64>, target_block_n: u64) -> TrieRevertAction {
    match latest_log_block_n {
        Some(current) if current > target_block_n => TrieRevertAction::Revert { current, target: target_block_n },
        Some(current) if current == target_block_n => TrieRevertAction::AlreadyAtTarget(current),
        Some(current) => TrieRevertAction::OlderThanTarget { current, target: target_block_n },
        None => TrieRevertAction::Missing,
    }
}

/// Reverts one Bonsai trie only when its logged head is newer than the target.
/// The boolean tells the caller whether that trie needs a new commit at the target revision.
pub(super) fn revert_single_trie<H: StarkHash + Send + Sync>(
    trie_name: &str,
    trie: &mut trie::GlobalTrie<H>,
    latest_log_block_n: Option<u64>,
    target_block_n: u64,
) -> anyhow::Result<bool> {
    match trie_revert_action(latest_log_block_n, target_block_n) {
        TrieRevertAction::Revert { current, target } => {
            tracing::debug!("🌳 REORG: Reverting {trie_name} trie from trie_head={} to target={}", current, target);
            trie.revert_to(BasicId::new(target), BasicId::new(current))
                .map_err(|e| anyhow::anyhow!("Failed to revert {trie_name} trie: {e:?}"))?;
            tracing::info!("✅ REORG: {trie_name} trie reverted successfully");
            Ok(true)
        }
        TrieRevertAction::AlreadyAtTarget(current) => {
            tracing::info!(
                "🌳 REORG: Skipping {trie_name} trie revert because trie_head={} already matches target={}",
                current,
                target_block_n
            );
            Ok(false)
        }
        TrieRevertAction::OlderThanTarget { current, target } => {
            tracing::info!(
                "🌳 REORG: Skipping {trie_name} trie revert because trie_head={} is older than target={}",
                current,
                target
            );
            Ok(false)
        }
        TrieRevertAction::Missing => {
            tracing::info!("🌳 REORG: Skipping {trie_name} trie revert because it has no persisted trie logs");
            Ok(false)
        }
    }
}

/// Verifies that a parallel-Merkle revert stays inside the configured trie-log window.
///
/// Boundary logs are sparse, but retention is expressed in block revisions. Reverting by
/// more blocks than the retained window could make Bonsai treat pruned revisions as empty
/// change sets and silently reconstruct the wrong trie state.
pub(super) fn ensure_parallel_merkle_revert_is_retained(
    latest_checkpoint: u64,
    target_block_n: u64,
    checkpoint_floor: u64,
    max_saved_trie_logs: Option<usize>,
) -> Result<()> {
    let Some(max_saved_trie_logs) = max_saved_trie_logs else {
        return Ok(());
    };
    let retained_block_revisions =
        u64::try_from(max_saved_trie_logs).context("Converting trie-log retention to u64")?;
    let first_retained_revision = if retained_block_revisions == 0 {
        latest_checkpoint.checked_add(1).context("Computing empty trie-log retention floor")?
    } else {
        latest_checkpoint.saturating_sub(retained_block_revisions - 1)
    };

    if checkpoint_floor < first_retained_revision {
        anyhow::bail!(
            "Cannot revert parallel Merkle checkpoint from block {latest_checkpoint} to {target_block_n}: checkpoint floor {checkpoint_floor} predates first retained trie-log revision {first_retained_revision} (window={retained_block_revisions})"
        );
    }

    Ok(())
}

/// Rejects a reorg result whose materialized trie root does not match the target block.
///
/// The caller must invoke this before advancing the persisted head projection.
pub(super) fn ensure_reorg_target_root_matches(
    target_block_n: u64,
    expected_root: Felt,
    actual_root: Felt,
) -> Result<()> {
    if actual_root != expected_root {
        anyhow::bail!(
            "Reorg target state root mismatch at block {target_block_n}: expected {expected_root:#x}, actual {actual_root:#x}; refusing to advance head projection"
        );
    }

    Ok(())
}

impl RocksDBStorage {
    /// Builds descriptors for every column family already present on disk.
    ///
    /// Known Madara columns keep their tuned options. Unknown columns are opened
    /// with RocksDB defaults and left untouched so a newer binary can still open
    /// databases written by older or experimental builds.
    fn column_family_descriptors(
        path: &Path,
        global_opts: &RocksDBOptions,
        config: &RocksDBConfig,
    ) -> Result<Vec<ColumnFamilyDescriptor>> {
        let mut descriptors: Vec<_> = ALL_COLUMNS
            .iter()
            .map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(config)))
            .collect();

        if path.join("CURRENT").exists() {
            for name in DB::list_cf(global_opts, path).context("Listing existing RocksDB column families")? {
                if name == "default" || ALL_COLUMNS.iter().any(|column| column.rocksdb_name == name) {
                    continue;
                }

                tracing::warn!(
                    column_family = name,
                    "Opening unknown RocksDB column family with default options to preserve compatibility"
                );
                descriptors.push(ColumnFamilyDescriptor::new(name, RocksDBOptions::default()));
            }
        }

        Ok(descriptors)
    }

    /// Opens Madara's RocksDB while preserving unknown legacy column families.
    pub fn open(path: &Path, config: RocksDBConfig) -> Result<Self> {
        let opts = rocksdb_global_options(&config)?;
        tracing::debug!("Opening db at {:?}", path.display());
        let descriptors = Self::column_family_descriptors(path, &opts, &config)?;
        let db = DB::open_cf_descriptors(&opts, path, descriptors)?;

        let writeopts = config.write_mode.to_write_options();
        tracing::info!("📝 Database write mode: {}", config.write_mode);
        let inner = Arc::new(RocksDBStorageInner { global_opts: opts, writeopts, db, config: config.clone() });

        let head_block_n = inner.get_head_projection_without_content()?.and_then(|c| match c {
            StoredHeadProjectionWithoutContent::Confirmed(block_n) => Some(block_n),
            StoredHeadProjectionWithoutContent::Preconfirmed(header) => header.block_number.checked_sub(1),
        });
        tracing::debug!(
            "opened_db_snapshot_config head_block_n={head_block_n:?} max_kept_snapshots={:?} snapshot_interval={}",
            config.max_kept_snapshots,
            config.snapshot_interval
        );

        let snapshot = Snapshots::new(inner.clone(), head_block_n, config.max_kept_snapshots, config.snapshot_interval);

        let storage = Self {
            inner,
            snapshots: snapshot.into(),
            metrics: DbMetrics::register().context("Registering database metrics")?,
            backup: BackupManager::start_if_enabled(path, &config).context("Startup backup manager")?,
        };

        if let Some(head_block_n) = head_block_n {
            if storage.has_parallel_merkle_checkpoint(head_block_n)? {
                storage.snapshots.pin_head(head_block_n);
            }
        }

        Ok(storage)
    }

    /// Flush all pending writes to disk. This is important when WAL is disabled.
    /// Should be called before shutdown to ensure data persistence.
    pub fn flush(&self) -> Result<()> {
        self.inner.flush()
    }

    /// Get a reference to the underlying RocksDB instance.
    ///
    /// This is primarily used for database migrations that need direct access
    /// to the raw DB for low-level operations.
    ///
    /// # Warning
    ///
    /// Direct manipulation of the DB can lead to data corruption if not done
    /// carefully. This should only be used by the migration system.
    pub fn inner_db(&self) -> &DB {
        &self.inner.db
    }

    /// Reads the latest persisted revision for every trie-log column.
    /// Reconciliation uses the three heads to detect partially applied boundaries.
    pub(super) fn trie_log_heads(&self) -> anyhow::Result<TrieLogHeads> {
        Ok(TrieLogHeads {
            contract: self.inner.latest_bonsai_log_id(trie::BONSAI_CONTRACT_LOG_COLUMN)?,
            contract_storage: self.inner.latest_bonsai_log_id(trie::BONSAI_CONTRACT_STORAGE_LOG_COLUMN)?,
            class: self.inner.latest_bonsai_log_id(trie::BONSAI_CLASS_LOG_COLUMN)?,
        })
    }

    /// Persists a durable parallel-Merkle checkpoint for one confirmed block.
    /// The underlying metadata layer updates both the marker and latest pointer.
    pub fn write_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<()> {
        self.inner.write_parallel_merkle_checkpoint(block_n)
    }

    /// Reports whether a durable checkpoint marker exists for the supplied block.
    /// This is a metadata lookup and does not validate the current trie root.
    pub fn has_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<bool> {
        self.inner.has_parallel_merkle_checkpoint(block_n)
    }

    /// Returns the newest checkpoint recorded by the metadata layer.
    /// Empty databases and databases without parallel-Merkle history return `None`.
    pub fn get_parallel_merkle_latest_checkpoint(&self) -> Result<Option<u64>> {
        self.inner.get_parallel_merkle_latest_checkpoint()
    }

    /// Finds the newest durable checkpoint at or below the requested block.
    /// Recovery can safely use the returned block as its replay base.
    pub fn get_parallel_merkle_checkpoint_floor(&self, target_block_n: u64) -> Result<Option<u64>> {
        self.inner.get_parallel_merkle_checkpoint_floor(target_block_n)
    }

    /// Removes checkpoint metadata newer than a canonical target block.
    /// The retained latest pointer is recomputed by the metadata layer.
    pub fn remove_parallel_merkle_checkpoints_above(&self, target_block_n: u64) -> Result<()> {
        self.inner.remove_parallel_merkle_checkpoints_above(target_block_n)
    }

    /// Removes checkpoint metadata newer than the supplied canonical target.
    /// `None` clears the checkpoint frontier back to an empty chain.
    fn rewind_parallel_merkle_checkpoints(&self, target_block_n: Option<u64>) -> Result<()> {
        self.inner.rewind_parallel_merkle_checkpoints(target_block_n)
    }

    /// Selects the newest runtime snapshot at or below the requested block.
    /// The empty-base snapshot is represented by a `None` block number.
    pub fn get_latest_snapshot_floor(&self, max_block_n: Option<u64>) -> Option<(Option<u64>, SnapshotRef)> {
        self.snapshots.get_floor(max_block_n)
    }

    /// Selects the newest durable snapshot at or below the requested block.
    /// Unlike runtime floors, this excludes uncommitted exact snapshots.
    pub fn get_latest_durable_snapshot_floor(&self, max_block_n: Option<u64>) -> Option<(Option<u64>, SnapshotRef)> {
        self.snapshots.get_durable_floor(max_block_n)
    }

    /// Warns when trie-log persistence disables reorg recovery for parallel Merkle.
    /// Root computation remains permitted so the configuration is non-fatal.
    pub fn ensure_parallel_merkle_recovery_config(&self) -> Result<()> {
        if self.inner.config.max_saved_trie_logs == Some(0) {
            tracing::warn!(
                "Parallel Merkle is running with trie-log persistence disabled; roots can still be computed, but trie-log-based reorg recovery is unavailable"
            );
        }
        Ok(())
    }

    /// Deletes every global-trie column entry to restore the empty durable base.
    /// All deletions share one RocksDB write batch.
    fn clear_global_trie_columns(&self) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        for column in [
            trie::BONSAI_CONTRACT_FLAT_COLUMN,
            trie::BONSAI_CONTRACT_TRIE_COLUMN,
            trie::BONSAI_CONTRACT_LOG_COLUMN,
            trie::BONSAI_CONTRACT_STORAGE_FLAT_COLUMN,
            trie::BONSAI_CONTRACT_STORAGE_TRIE_COLUMN,
            trie::BONSAI_CONTRACT_STORAGE_LOG_COLUMN,
            trie::BONSAI_CLASS_FLAT_COLUMN,
            trie::BONSAI_CLASS_TRIE_COLUMN,
            trie::BONSAI_CLASS_LOG_COLUMN,
        ] {
            let handle = self.inner.get_column(column);
            for item in self.inner.db.iterator_cf(&handle, IteratorMode::Start) {
                let (key, _value) = item?;
                batch.delete_cf(&handle, key);
            }
        }
        self.inner.db.write_opt(batch, &self.inner.writeopts)?;
        Ok(())
    }

    /// Rolls all three tries back to a common checkpoint floor or the empty base.
    /// The result reports whether any durable trie state changed.
    fn rollback_tries_to_checkpoint_floor(&self, checkpoint_floor: Option<u64>, context: &str) -> Result<bool> {
        let trie_log_heads = self.trie_log_heads().context("Reading trie log heads before recovery rollback")?;

        let Some(checkpoint_floor) = checkpoint_floor else {
            let had_durable_trie_state =
                trie_log_heads.highest().is_some() || self.get_state_root_hash()? != Felt::ZERO;
            if had_durable_trie_state {
                tracing::warn!(
                    "parallel_merkle_recovery_rollback_to_empty context={} trie_log_heads={:?}",
                    context,
                    trie_log_heads
                );
                self.clear_global_trie_columns().context("Clearing global tries back to the empty durable base")?;
            }
            return Ok(had_durable_trie_state);
        };

        let floor_id = BasicId::new(checkpoint_floor);
        let mut contract_trie = self.contract_trie_for_revert();
        let contract_needs_commit =
            revert_single_trie("contract", &mut contract_trie, trie_log_heads.contract, checkpoint_floor)?;
        let mut contract_storage_trie = self.contract_storage_trie_for_revert();
        let contract_storage_needs_commit = revert_single_trie(
            "contract storage",
            &mut contract_storage_trie,
            trie_log_heads.contract_storage,
            checkpoint_floor,
        )?;
        let mut class_trie = self.class_trie_for_revert();
        let class_needs_commit = revert_single_trie("class", &mut class_trie, trie_log_heads.class, checkpoint_floor)?;

        if contract_needs_commit {
            contract_trie.commit(floor_id).map_err(trie::WrappedBonsaiError)?;
        }
        if contract_storage_needs_commit {
            contract_storage_trie.commit(floor_id).map_err(trie::WrappedBonsaiError)?;
        }
        if class_needs_commit {
            class_trie.commit(floor_id).map_err(trie::WrappedBonsaiError)?;
        }

        Ok(contract_needs_commit || contract_storage_needs_commit || class_needs_commit)
    }

    /// Selects a checkpoint that every materialized trie can reach by rolling backward.
    ///
    /// During an interrupted reorg, one trie may already be older than the confirmed tip.
    /// Starting from the oldest live revision lets recovery align all tries at one durable
    /// checkpoint before replaying confirmed state diffs forward.
    fn confirmed_recovery_checkpoint_floor(
        &self,
        confirmed_tip: u64,
        trie_log_heads: TrieLogHeads,
    ) -> Result<Option<u64>> {
        let Some(oldest_trie_head) = trie_log_heads.lowest() else {
            return Ok(None);
        };
        self.get_parallel_merkle_checkpoint_floor(oldest_trie_head.min(confirmed_tip))
    }

    /// Reconciles an empty canonical head with empty trie, checkpoint, and snapshot state.
    ///
    /// Any materialized future state is removed before the empty root invariant is checked.
    fn reconcile_empty_confirmed_head(&self, context: &str) -> Result<()> {
        let latest_checkpoint = self.get_parallel_merkle_latest_checkpoint()?;
        let trie_log_heads = self.trie_log_heads()?;
        let actual_root = self.get_state_root_hash()?;
        if latest_checkpoint.is_some() || trie_log_heads.highest().is_some() || actual_root != Felt::ZERO {
            self.rollback_tries_to_checkpoint_floor(None, context)?;
            self.rewind_parallel_merkle_checkpoints(None)?;
        }

        let reconciled_root = self.get_state_root_hash()?;
        ensure!(
            reconciled_root == Felt::ZERO,
            "Empty confirmed head must have an empty trie after {context}, got {reconciled_root:#x}"
        );
        self.write_latest_applied_trie_update(&None)?;
        self.snapshots.rewind_to_empty();
        Ok(())
    }

    /// Loads the confirmed block metadata required to verify and rebuild its state root.
    ///
    /// Missing canonical block information is fatal because no authoritative root is available.
    fn confirmed_block_for_reconcile(&self, confirmed_tip: u64) -> Result<MadaraBlockInfo> {
        self.inner
            .get_block_info(confirmed_tip)
            .with_context(|| {
                format!("Reading block info for confirmed block #{confirmed_tip} during parallel merkle reconciliation")
            })?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing block info for confirmed block #{confirmed_tip} during parallel merkle reconciliation"
                )
            })
    }

    /// Resolves the expected root at a durable checkpoint floor.
    ///
    /// The absent floor represents the empty trie and therefore has the zero root.
    fn checkpoint_floor_root(&self, checkpoint_floor: Option<u64>) -> Result<Felt> {
        let Some(checkpoint_floor) = checkpoint_floor else {
            return Ok(Felt::ZERO);
        };
        self.inner
            .get_block_info(checkpoint_floor)
            .with_context(|| {
                format!(
                    "Reading block info for durable checkpoint floor #{checkpoint_floor} during parallel merkle reconciliation"
                )
            })?
            .map(|info| info.header.global_state_root)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing block info for durable checkpoint floor #{checkpoint_floor} during parallel merkle reconciliation"
                )
            })
    }

    /// Verifies the live trie root immediately after rolling back to a durable floor.
    ///
    /// This check prevents replaying confirmed diffs on top of an already-corrupt base.
    fn verify_checkpoint_floor_root(
        &self,
        checkpoint_floor: Option<u64>,
        expected_floor_root: Felt,
        confirmed_tip: u64,
        context: &str,
    ) -> Result<()> {
        let actual_floor_root = match checkpoint_floor {
            Some(block_n) => {
                let protocol_version = self
                    .inner
                    .get_block_info(block_n)?
                    .context("Missing checkpoint floor block info after recovery rollback")?
                    .header
                    .protocol_version;
                get_state_root(self, protocol_version)?
            }
            None => self.get_state_root_hash()?,
        };
        ensure!(
            actual_floor_root == expected_floor_root,
            "Trie root {actual_floor_root:#x} does not match durable checkpoint floor {checkpoint_floor:?} root {expected_floor_root:#x} while reconciling confirmed block #{confirmed_tip} during {context}"
        );
        Ok(())
    }

    /// Rolls back to a common checkpoint and replays ordered diffs through the confirmed tip.
    ///
    /// Returns whether rollback and replay performed work for reconciliation telemetry.
    fn rebuild_confirmed_trie(
        &self,
        confirmed_tip: u64,
        confirmed_block_info: &MadaraBlockInfo,
        trie_log_heads: TrieLogHeads,
        context: &str,
    ) -> Result<(bool, bool)> {
        let checkpoint_floor =
            self.confirmed_recovery_checkpoint_floor(confirmed_tip, trie_log_heads).with_context(|| {
                format!("Reading common parallel merkle recovery checkpoint for confirmed block #{confirmed_tip}")
            })?;
        let expected_floor_root = self.checkpoint_floor_root(checkpoint_floor)?;
        let rolled_back = self.rollback_tries_to_checkpoint_floor(checkpoint_floor, context)?;
        self.rewind_parallel_merkle_checkpoints(checkpoint_floor)?;
        self.verify_checkpoint_floor_root(checkpoint_floor, expected_floor_root, confirmed_tip, context)?;

        let replay_start = checkpoint_floor.map_or(0, |block_n| block_n + 1);
        let replayed = replay_start <= confirmed_tip;
        if replayed {
            self.replay_state_diffs_inclusive(
                replay_start,
                confirmed_tip,
                confirmed_block_info.header.protocol_version,
                "parallel merkle reconciliation",
            )
            .with_context(|| format!("Rebuilding confirmed block #{confirmed_tip} during {context}"))?;
        }
        Ok((rolled_back, replayed))
    }

    /// Persists authoritative recovery metadata and refreshes the snapshot inventory.
    ///
    /// Returns whether a missing checkpoint marker had to be created.
    fn finalize_confirmed_reconcile(&self, confirmed_tip: u64) -> Result<bool> {
        self.write_latest_applied_trie_update(&Some(confirmed_tip)).with_context(|| {
            format!("Writing latest_applied_trie_update={confirmed_tip} during parallel merkle reconciliation")
        })?;

        let wrote_checkpoint = !self.has_parallel_merkle_checkpoint(confirmed_tip).with_context(|| {
            format!("Checking checkpoint for confirmed block #{confirmed_tip} during parallel merkle reconciliation")
        })?;
        if wrote_checkpoint {
            self.write_parallel_merkle_checkpoint(confirmed_tip).with_context(|| {
                format!("Writing checkpoint for confirmed block #{confirmed_tip} during parallel merkle reconciliation")
            })?;
        }
        self.on_new_confirmed_head(confirmed_tip).with_context(|| {
            format!("Refreshing snapshot inventory for confirmed block #{confirmed_tip} during parallel merkle reconciliation")
        })?;
        Ok(wrote_checkpoint)
    }

    /// Reconciles materialized parallel-Merkle state to the authoritative confirmed head.
    ///
    /// Recovery rolls back to a common durable floor, replays ordered diffs, and republishes metadata.
    pub fn reconcile_confirmed_parallel_merkle_state(&self, confirmed_tip: Option<u64>, context: &str) -> Result<()> {
        let Some(confirmed_tip) = confirmed_tip else {
            return self.reconcile_empty_confirmed_head(context);
        };

        let confirmed_block_info = self.confirmed_block_for_reconcile(confirmed_tip)?;
        let expected_root = confirmed_block_info.header.global_state_root;
        let actual_root = get_state_root(self, confirmed_block_info.header.protocol_version).with_context(|| {
            format!(
                "Reading global state root for confirmed block #{confirmed_tip} during parallel merkle reconciliation"
            )
        })?;
        tracing::debug!(
            "parallel_merkle_confirmed_reconcile_start context={} confirmed_tip={} expected_root={:#x} actual_root={:#x}",
            context,
            confirmed_tip,
            expected_root,
            actual_root
        );

        let latest_checkpoint = self.get_parallel_merkle_latest_checkpoint()?;
        let trie_log_heads = self.trie_log_heads()?;
        let durable_state_is_ahead = latest_checkpoint.is_some_and(|checkpoint| checkpoint > confirmed_tip)
            || trie_log_heads.highest().is_some_and(|trie_head| trie_head > confirmed_tip);
        let (rolled_back_to_floor, replayed_from_floor) = if actual_root != expected_root || durable_state_is_ahead {
            self.rebuild_confirmed_trie(confirmed_tip, &confirmed_block_info, trie_log_heads, context)?
        } else {
            (false, false)
        };

        let reconciled_root =
            get_state_root(self, confirmed_block_info.header.protocol_version).with_context(|| {
                format!("Reading global state root after reconciliation for confirmed block #{confirmed_tip} during {context}")
            })?;
        ensure!(
            reconciled_root == expected_root,
            "Confirmed block #{confirmed_tip} root mismatch after {context}: expected {expected_root:#x}, got {reconciled_root:#x}"
        );
        let wrote_checkpoint = self.finalize_confirmed_reconcile(confirmed_tip)?;

        let log_message = "parallel_merkle_confirmed_reconcile_complete";
        if rolled_back_to_floor || replayed_from_floor || wrote_checkpoint {
            tracing::info!(
                context,
                confirmed_tip,
                state_root = format_args!("{reconciled_root:#x}"),
                rolled_back_to_floor,
                replayed_from_floor,
                wrote_checkpoint,
                "{log_message}"
            );
        } else {
            tracing::debug!(
                context,
                confirmed_tip,
                state_root = format_args!("{reconciled_root:#x}"),
                rolled_back_to_floor,
                replayed_from_floor,
                wrote_checkpoint,
                "{log_message}"
            );
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// Computes a root from an explicitly selected snapshot and optionally cross-checks sequentially.
    /// The comparison path logs mismatches but returns the parallel computation unchanged.
    pub fn compute_root_from_selected_snapshot(
        &self,
        snapshot_block: Option<u64>,
        snapshot: SnapshotRef,
        block_n: u64,
        state_diff: &StateDiff,
        protocol_version: mp_chain_config::StarknetVersion,
        include_overlay: bool,
        compare_with_sequential: bool,
    ) -> Result<InMemoryRootComputation> {
        tracing::debug!(
            "parallel_root_selected_snapshot_compute block_number={} base_block={snapshot_block:?} include_overlay={}",
            block_n,
            include_overlay
        );
        let compare_snapshot = compare_with_sequential.then(|| Arc::clone(&snapshot));
        let parallel = compute_root_from_snapshot(
            self,
            snapshot_block,
            snapshot,
            block_n,
            state_diff,
            protocol_version,
            include_overlay,
        )?;

        if let Some(compare_snapshot) = compare_snapshot {
            let sequential = compute_root_from_snapshot_sequential(
                self,
                snapshot_block,
                compare_snapshot,
                block_n,
                state_diff,
                protocol_version,
            )?;
            tracing::debug!(
                "parallel_vs_sequential_root_compare block_number={} base_block={snapshot_block:?} contract_root_parallel={:#x} contract_root_sequential={:#x} contract_root_match={} class_root_parallel={:#x} class_root_sequential={:#x} class_root_match={} state_root_parallel={:#x} state_root_sequential={:#x} state_root_match={} include_overlay={}",
                block_n,
                parallel.contract_root,
                sequential.contract_root,
                parallel.contract_root == sequential.contract_root,
                parallel.class_root,
                sequential.class_root,
                parallel.class_root == sequential.class_root,
                parallel.state_root,
                sequential.state_root,
                parallel.state_root == sequential.state_root,
                include_overlay
            );
        }

        Ok(parallel)
    }

    /// Computes one root from the exact snapshot of its immediate parent block.
    /// Missing exact snapshots fail with inventory diagnostics instead of falling back silently.
    pub fn compute_root_from_latest_snapshot(
        &self,
        block_n: u64,
        state_diff: &StateDiff,
        protocol_version: mp_chain_config::StarknetVersion,
        include_overlay: bool,
    ) -> Result<InMemoryRootComputation> {
        let base_block_n = block_n.checked_sub(1);
        let inventory = self.snapshots.inventory();
        let snapshot = self.snapshots.get_exact(base_block_n).ok_or_else(|| {
            tracing::error!(
                "parallel_root_base_snapshot_missing block_number={} base_block={base_block_n:?} head_block_n={:?} exact_count={} historical_count={} oldest_exact={:?} newest_exact={:?} oldest_snapshot={:?} newest_snapshot={:?} has_empty_base={} latest_checkpoint={:?} checkpoint_floor={:?} include_overlay={}",
                block_n,
                inventory.head_block_n,
                inventory.exact_count,
                inventory.historical_count,
                inventory.oldest_exact,
                inventory.newest_exact,
                inventory.oldest_historical,
                inventory.newest_historical,
                inventory.has_empty_base,
                self.get_parallel_merkle_latest_checkpoint().ok().flatten(),
                self.get_parallel_merkle_checkpoint_floor(block_n).ok().flatten(),
                include_overlay
            );
            anyhow::anyhow!("Missing exact base snapshot for block #{block_n} (base {base_block_n:?})")
        })?;
        tracing::debug!(
            "parallel_root_base_snapshot_selected block_number={} base_block={base_block_n:?} snapshot_block={base_block_n:?} exact_match=true latest_checkpoint={:?} checkpoint_floor={:?} include_overlay={}",
            block_n,
            self.get_parallel_merkle_latest_checkpoint().ok().flatten(),
            self.get_parallel_merkle_checkpoint_floor(block_n).ok().flatten(),
            include_overlay
        );
        compute_root_from_snapshot(self, base_block_n, snapshot, block_n, state_diff, protocol_version, include_overlay)
    }

    /// Computes a contiguous root batch from the exact snapshot before its first block.
    /// Boundary selection controls which result retains a flushable overlay.
    pub fn compute_roots_in_parallel_from_latest_snapshot(
        &self,
        start_block_n: u64,
        state_diffs: &[StateDiff],
        protocol_version: mp_chain_config::StarknetVersion,
        boundary_block_n: Option<u64>,
    ) -> Result<Vec<InMemoryRootComputation>> {
        let base_block_n = start_block_n.checked_sub(1);
        let end_block_n = start_block_n
            + u64::try_from(state_diffs.len().saturating_sub(1)).expect("state diff batch size fits in u64");
        let inventory = self.snapshots.inventory();
        let snapshot = self.snapshots.get_exact(base_block_n).ok_or_else(|| {
            tracing::error!(
                "parallel_root_base_snapshot_missing start_block={} end_block={} batch_size={} base_block={base_block_n:?} boundary_block={boundary_block_n:?} head_block_n={:?} exact_count={} historical_count={} oldest_exact={:?} newest_exact={:?} oldest_snapshot={:?} newest_snapshot={:?} has_empty_base={} latest_checkpoint={:?} checkpoint_floor_for_start={:?}",
                start_block_n,
                end_block_n,
                state_diffs.len(),
                inventory.head_block_n,
                inventory.exact_count,
                inventory.historical_count,
                inventory.oldest_exact,
                inventory.newest_exact,
                inventory.oldest_historical,
                inventory.newest_historical,
                inventory.has_empty_base,
                self.get_parallel_merkle_latest_checkpoint().ok().flatten(),
                self.get_parallel_merkle_checkpoint_floor(start_block_n).ok().flatten()
            );
            anyhow::anyhow!(
                "Missing exact base snapshot for root batch {}..={} (base {base_block_n:?})",
                start_block_n,
                end_block_n
            )
        })?;
        tracing::debug!(
            "parallel_root_base_snapshot_selected start_block={} end_block={} batch_size={} base_block={base_block_n:?} snapshot_block={base_block_n:?} exact_match=true latest_checkpoint={:?} checkpoint_floor_for_start={:?} boundary_block={boundary_block_n:?}",
            start_block_n,
            end_block_n,
            state_diffs.len(),
            self.get_parallel_merkle_latest_checkpoint().ok().flatten(),
            self.get_parallel_merkle_checkpoint_floor(start_block_n).ok().flatten()
        );
        compute_roots_in_parallel_from_snapshot(
            self,
            base_block_n,
            snapshot,
            start_block_n,
            state_diffs,
            protocol_version,
            boundary_block_n,
        )
    }

    /// Publishes a boundary overlay only when its selected base is still durable.
    /// Stale work is reported through `BoundaryFlushOutcome` without overwriting newer trie state.
    pub fn flush_overlay_and_checkpoint(
        &self,
        block_n: u64,
        boundary_interval: u64,
        overlay_base_block_n: Option<u64>,
        overlay: &BonsaiOverlay,
    ) -> Result<crate::rocksdb::global_trie::in_memory::BoundaryFlushOutcome> {
        crate::rocksdb::global_trie::in_memory::flush_overlay_and_checkpoint(
            self,
            block_n,
            boundary_interval,
            overlay_base_block_n,
            overlay,
        )
    }

    /// Loads a complete inclusive state-diff range in ascending block order.
    /// Missing rows fail before any replay mutation begins.
    fn collect_state_diffs_inclusive(&self, from_block_n: u64, to_block_n: u64) -> Result<Vec<(u64, StateDiff)>> {
        if from_block_n > to_block_n {
            return Ok(Vec::new());
        }

        let mut diffs = Vec::with_capacity((to_block_n - from_block_n + 1) as usize);
        for block_n in from_block_n..=to_block_n {
            let state_diff = self
                .inner
                .get_block_state_diff(block_n)
                .with_context(|| format!("Reading state diff for block #{block_n} during reorg replay"))?
                .ok_or_else(|| anyhow::anyhow!("Missing state diff for block #{block_n} during reorg replay"))?;
            diffs.push((block_n, state_diff));
        }

        Ok(diffs)
    }

    /// Reapplies stored state diffs in block order and materializes the target trie revision.
    ///
    /// State diffs must remain ordered because collapsing a long range can lose transitions
    /// whose meaning depends on state established by an earlier block in that range.
    pub(super) fn replay_state_diffs_inclusive(
        &self,
        from_block_n: u64,
        to_block_n: u64,
        protocol_version: StarknetVersion,
        operation: &str,
    ) -> Result<()> {
        let replay_diffs = self
            .collect_state_diffs_inclusive(from_block_n, to_block_n)
            .with_context(|| format!("Collecting state diffs {from_block_n}..={to_block_n} for {operation}"))?;
        self.apply_to_global_trie(
            from_block_n,
            replay_diffs.iter().map(|(_, state_diff)| state_diff),
            protocol_version,
        )
        .with_context(|| format!("Applying state diffs {from_block_n}..={to_block_n} for {operation}"))?;
        Ok(())
    }

    /// Atomically publishes the canonical reorg target and its coupled recovery metadata.
    ///
    /// Before this batch commits, the old confirmed head remains authoritative and every
    /// future block/state-diff row is still available for startup trie reconstruction. After
    /// it commits, startup can treat `target_block_n` as authoritative and resume deletion of
    /// the now-noncanonical suffix without exposing an intermediate head.
    pub(super) fn commit_reorg_head(
        &self,
        target_block_n: u64,
        l1_message_nonces_to_cleanup: &[u64],
        l1_messaging_sync_tip_after_revert: Option<u64>,
    ) -> Result<()> {
        let mut batch = WriteBatchWithTransaction::default();
        self.inner.replace_head_projection_in_batch(&StorageHeadProjection::Confirmed(target_block_n), &mut batch)?;
        self.inner.delete_all_preconfirmed_rows_in_batch(&mut batch)?;
        self.inner.message_to_l2_remove_for_nonces(l1_message_nonces_to_cleanup, &mut batch)?;
        self.inner.write_latest_applied_trie_update_in_batch(&Some(target_block_n), &mut batch)?;
        if let Some(l1_sync_tip) = l1_messaging_sync_tip_after_revert {
            self.inner.write_l1_messaging_sync_tip_in_batch(Some(l1_sync_tip), &mut batch);
        }
        self.inner.db.write_opt(batch, &self.inner.writeopts).context("Committing canonical reorg head")
    }
}

#[cfg(test)]
mod tests;
