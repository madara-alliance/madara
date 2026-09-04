use super::*;

impl<D: MadaraStorage> MadaraBackend<D> {
    /// Compares the live trie root with the root stored in one confirmed header.
    /// A mismatch is logged and returned as false so reconciliation can fail with context.
    fn log_confirmed_state_root_consistency(&self, context: &str, block_n: u64) -> Result<bool> {
        let block_info = self.db.get_block_info(block_n)?.ok_or_else(|| {
            anyhow::anyhow!("Missing block info while verifying confirmed state root for block #{block_n}")
        })?;
        let expected_root = block_info.header.global_state_root;
        let actual_root = self.db.get_state_root_hash_at_version(block_info.header.protocol_version)?;
        let matches = actual_root == expected_root;
        tracing::info!(
            "confirmed_state_root_consistency context={} block_number={} expected_root={:#x} actual_root={:#x} match={}",
            context,
            block_n,
            expected_root,
            actual_root,
            matches
        );
        if !matches {
            tracing::error!(
                "confirmed_state_root_consistency_mismatch context={} block_number={} expected_root={:#x} actual_root={:#x}",
                context,
                block_n,
                expected_root,
                actual_root
            );
        }
        Ok(matches)
    }

    /// Reconciles trie durability to the supplied confirmed tip and verifies its resulting root.
    /// Empty heads require only backend reconciliation and skip block-root verification.
    fn reconcile_confirmed_parallel_merkle_state_for_tip(
        &self,
        confirmed_tip: Option<u64>,
        context: &str,
    ) -> Result<()> {
        self.db
            .reconcile_confirmed_parallel_merkle_state(confirmed_tip, context)
            .with_context(|| format!("Reconciling trie state for confirmed tip {confirmed_tip:?} during {context}"))?;

        if let Some(confirmed_tip) = confirmed_tip {
            ensure!(
                self.log_confirmed_state_root_consistency(context, confirmed_tip)?,
                "Confirmed state root mismatch after parallel merkle reconciliation for block #{confirmed_tip} during {context}"
            );
        }

        Ok(())
    }

    /// Constructs the backend's synchronization primitives and initializes persisted state.
    /// The backend is returned only after recovery and chain-identity checks succeed.
    fn new_and_init(
        db: D,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
        cairo_native_config: Arc<NativeConfig>,
    ) -> Result<Self> {
        let (reorg_notifications, _) = tokio::sync::broadcast::channel(16);
        let mut backend = Self {
            db,
            // db_metrics: DbMetrics::register().context("Registering db metrics")?,
            chain_config,
            starting_block: config.unsafe_starting_block,
            config,
            sync_status: SyncStatusCell::default(),
            watch_gas_quote: L1GasQuoteCell::default(),
            cairo_native_config,
            #[cfg(any(test, feature = "testing"))]
            _temp_dir: None,
            chain_head_state: tokio::sync::watch::Sender::new(Default::default()),
            preconfirmed_block_runtime: RwLock::new(BTreeMap::new()),
            head_projection_write_lock: Mutex::new(()),
            latest_l1_confirmed: tokio::sync::watch::Sender::new(Default::default()),
            reorg_notifications,
            custom_headers: Mutex::new(Default::default()),
            replay_boundaries: Mutex::new(BTreeMap::new()),
        };
        backend.init().context("Initializing madara backend")?;
        Ok(backend)
    }

    /// Validates chain identity, cleans crash leftovers, reconciles tries, and publishes runtime heads.
    /// The confirmed projection remains authoritative throughout startup recovery.
    fn init(&mut self) -> Result<()> {
        // Check chain configuration
        if let Some(res) = self.db.get_stored_chain_info()? {
            if res.chain_id != self.chain_config.chain_id {
                bail!(
                    "The database has been created on the network \"{}\" (chain id `{}`), \
                            but the node is configured for network \"{}\" (chain id `{}`).",
                    res.chain_name,
                    res.chain_id,
                    self.chain_config.chain_name,
                    self.chain_config.chain_id
                )
            }
        } else {
            self.db.write_chain_info(&StoredChainInfo {
                chain_id: self.chain_config.chain_id.clone(),
                chain_name: self.chain_config.chain_name.clone(),
            })?;
        }

        // Initialize canonical chain head state.
        let stored_tip = if let Some(starting_block) = self.starting_block {
            StorageHeadProjection::Confirmed(starting_block)
        } else {
            self.db.get_head_projection()?
        };
        let stored_confirmed_tip = ChainHeadState::from_head_projection(&stored_tip).confirmed_tip;
        if let Some(confirmed_tip) = stored_confirmed_tip {
            // A crash can happen after the durable head projection advances but before
            // confirmed-path preconfirmed GC. Remove those stale rows before rebuilding
            // the runtime head projection so they cannot be mistaken for a future tip.
            self.db.delete_preconfirmed_rows_up_to(confirmed_tip).with_context(|| {
                format!("Cleaning stale preconfirmed rows through confirmed block #{confirmed_tip}")
            })?;
        }
        let (chain_head_state, preconfirmed) = self.build_runtime_head_projection(stored_tip)?;
        self.starting_block = chain_head_state.confirmed_tip;
        // On startup, remove all blocks past the head projection, in case we have partial blocks in db.
        self.db.remove_all_blocks_starting_from(
            chain_head_state.confirmed_tip.map(|n| n + 1).unwrap_or(/* genesis */ 0),
        )?;
        self.reconcile_confirmed_parallel_merkle_state_for_tip(chain_head_state.confirmed_tip, "startup_init")?;
        if let Some(confirmed_tip) = chain_head_state.confirmed_tip {
            // A crash can happen after the durable head transition but before the derived L1
            // consumed/pending projection is updated. Re-applying the confirmed tip is safe and
            // sufficient because block confirmation is serialized.
            self.db
                .confirm_l1_messages_in_block(confirmed_tip)
                .with_context(|| format!("Repairing L1 message confirmation for block #{confirmed_tip}"))?;
        }
        self.publish_head_projection(chain_head_state, preconfirmed)?;

        // Init L1 head
        self.latest_l1_confirmed.send_replace(self.db.get_confirmed_on_l1_tip()?);

        Ok(())
    }

    /// Get a write handle for the backend. This is the function you need to call to save new blocks, modify the preconfirmed block,
    /// and do any other such thing. The canonical chain head projection can only be modified through this.
    ///
    /// Canonical head-projection transitions are serialized internally. Callers must still submit
    /// those transitions in canonical block order. Low-level `write_*` block-part functions may be
    /// used concurrently.
    ///
    /// Failure to do so could result in errors and/or invalid state, which includes invalid state being saved to the database.
    /// The functions are still safe to use, since it's a logic error and not a memory safety issue.
    ///
    /// In addition, all the associated functions need to be called in a rayon thread pool context. **Do not call
    /// them from the tokio pool!**
    pub fn write_access(self: &Arc<Self>) -> MadaraBackendWriter<D> {
        MadaraBackendWriter { inner: self.clone() }
    }

    /// Set the current latest block confirmed on L1. This will also wake watchers to L1 head changes.
    ///
    /// Warning: It is invalid to set this new `latest_l1_confirmed` to a lower value than the current one, or
    /// to a higher value than the current block on l2.
    // FIXME: In these cases, the update should not succeed and an error should be returned.
    pub fn set_latest_l1_confirmed(&self, latest_l1_confirmed: Option<u64>) -> Result<()> {
        self.db.write_confirmed_on_l1_tip(latest_l1_confirmed)?;
        self.latest_l1_confirmed.send_replace(latest_l1_confirmed);
        Ok(())
    }

    /// Derives replay progress from the runtime block first, then durable preconfirmed storage.
    /// Read failures are logged and conservatively produce an empty seed.
    fn replay_boundary_seed_from_preconfirmed(&self, block_n: u64) -> (u64, Option<Felt>) {
        if let Some(runtime_block) =
            runtime_preconfirmed_block(&self.preconfirmed_block_runtime.read().expect("Poisoned lock"), block_n)
        {
            let guard = runtime_block.content.borrow();
            let executed_tx_count = guard.n_executed() as u64;
            let last_executed_tx_hash =
                guard.executed_transactions().last().map(|tx| *tx.transaction.receipt.transaction_hash());
            return (executed_tx_count, last_executed_tx_hash);
        }

        match self.db.get_preconfirmed_block_data(block_n) {
            Ok(Some((_header, content))) => {
                let executed_tx_count = content.len() as u64;
                let last_executed_tx_hash = content.last().map(|tx| *tx.transaction.receipt.transaction_hash());
                (executed_tx_count, last_executed_tx_hash)
            }
            Ok(None) => (0, None),
            Err(err) => {
                tracing::warn!(
                    "Failed to read preconfirmed data while seeding replay boundary for block #{block_n}: {err:#}"
                );
                (0, None)
            }
        }
    }

    /// Creates or replaces one replay boundary and seeds it from already executed transactions.
    /// Reapplying an identical boundary preserves monotonic progress while reopening close state.
    pub fn set_replay_boundary(&self, boundary: ReplayBlockBoundary) -> ReplayBlockBoundaryStatus {
        let (seed_executed, seed_last_hash) = self.replay_boundary_seed_from_preconfirmed(boundary.block_n);
        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");

        if let Some(existing) = guard.get_mut(&boundary.block_n) {
            if existing.boundary == boundary {
                if seed_executed > existing.executed_tx_count {
                    existing.executed_tx_count = seed_executed;
                    existing.last_executed_tx_hash = seed_last_hash.or(existing.last_executed_tx_hash);
                }
                existing.dispatched_tx_count = existing.dispatched_tx_count.max(seed_executed);
                if existing.last_executed_tx_hash.is_none() {
                    existing.last_executed_tx_hash = seed_last_hash;
                }
                existing.reached_last_tx_hash =
                    existing.last_executed_tx_hash.map(|hash| hash == existing.boundary.last_tx_hash).unwrap_or(false);
                existing.closed = false;
                existing.refresh_consistency_flags();
                let status = existing.to_status();
                tracing::info!(
                    "replay_boundary_set block_number={} expected_tx_count={} seeded_executed={} boundary_met={} closed={} mismatch={:?}",
                    status.block_n,
                    status.expected_tx_count,
                    status.executed_tx_count,
                    status.boundary_met,
                    status.closed,
                    status.mismatch
                );
                return status;
            }

            tracing::warn!(
                "Replacing replay boundary for block #{} (old expected_tx_count={}, old_last_tx_hash={:#x}, new expected_tx_count={}, new_last_tx_hash={:#x})",
                boundary.block_n,
                existing.boundary.expected_tx_count,
                existing.boundary.last_tx_hash,
                boundary.expected_tx_count,
                boundary.last_tx_hash
            );
        }

        let runtime = ReplayBoundaryRuntime::from_boundary(boundary.clone(), seed_executed, seed_last_hash);
        let status = runtime.to_status();
        guard.insert(boundary.block_n, runtime);
        tracing::info!(
            "replay_boundary_set block_number={} expected_tx_count={} seeded_executed={} boundary_met={} closed={} mismatch={:?}",
            status.block_n,
            status.expected_tx_count,
            status.executed_tx_count,
            status.boundary_met,
            status.closed,
            status.mismatch
        );
        status
    }

    /// Returns a snapshot of the requested replay boundary's current counters and flags.
    /// Missing block entries remain distinguishable as `None`.
    pub fn get_replay_boundary_status(&self, block_n: u64) -> Option<ReplayBlockBoundaryStatus> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(ReplayBoundaryRuntime::to_status)
    }

    /// Reports whether replay control has a boundary registered for this block.
    /// The lookup holds the replay-boundary mutex only for the map read.
    pub fn replay_boundary_exists(&self, block_n: u64) -> bool {
        self.replay_boundaries.lock().expect("Poisoned lock").contains_key(&block_n)
    }

    /// Returns how many more transactions the block may execute before meeting its boundary.
    /// Closed or inconsistent boundaries expose zero remaining capacity.
    pub fn replay_boundary_remaining_execution_capacity(&self, block_n: u64) -> Option<u64> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(|entry| {
            if entry.closed || entry.mismatch.is_some() {
                0
            } else {
                entry.boundary.expected_tx_count.saturating_sub(entry.executed_tx_count)
            }
        })
    }

    /// Returns how many more transactions the batcher may dispatch for this boundary.
    /// Closed or inconsistent boundaries expose zero remaining capacity.
    pub fn replay_boundary_remaining_dispatch_capacity(&self, block_n: u64) -> Option<u64> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(|entry| {
            if entry.closed || entry.mismatch.is_some() {
                0
            } else {
                entry.boundary.expected_tx_count.saturating_sub(entry.dispatched_tx_count)
            }
        })
    }

    /// Reports whether executed count and final hash satisfy the requested replay boundary.
    /// A missing boundary remains `None` rather than being treated as unmet.
    pub fn replay_boundary_is_met(&self, block_n: u64) -> Option<bool> {
        self.replay_boundaries.lock().expect("Poisoned lock").get(&block_n).map(ReplayBoundaryRuntime::boundary_met)
    }

    /// Advances the dispatched count and records overflow as a permanent boundary mismatch.
    /// Zero-count calls are read-only status lookups.
    pub fn replay_boundary_record_dispatched(
        &self,
        block_n: u64,
        dispatched_tx_count: u64,
    ) -> Option<ReplayBlockBoundaryStatus> {
        if dispatched_tx_count == 0 {
            return self.get_replay_boundary_status(block_n);
        }

        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");
        let entry = guard.get_mut(&block_n)?;
        if entry.closed {
            return Some(entry.to_status());
        }

        entry.dispatched_tx_count = entry.dispatched_tx_count.saturating_add(dispatched_tx_count);
        if entry.dispatched_tx_count > entry.boundary.expected_tx_count {
            entry.set_mismatch_if_empty(format!(
                "dispatched_tx_count={} exceeded expected_tx_count={}",
                entry.dispatched_tx_count, entry.boundary.expected_tx_count
            ));
        }
        entry.refresh_consistency_flags();
        Some(entry.to_status())
    }

    /// Records executed hashes in order and updates count, last-hash, and consistency flags.
    /// Dispatch progress is raised to executed progress if recovery observes execution first.
    pub fn replay_boundary_record_executed_hashes(
        &self,
        block_n: u64,
        tx_hashes: &[Felt],
    ) -> Option<ReplayBlockBoundaryStatus> {
        if tx_hashes.is_empty() {
            return self.get_replay_boundary_status(block_n);
        }

        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");
        let entry = guard.get_mut(&block_n)?;
        for tx_hash in tx_hashes {
            entry.executed_tx_count = entry.executed_tx_count.saturating_add(1);
            if entry.dispatched_tx_count < entry.executed_tx_count {
                entry.dispatched_tx_count = entry.executed_tx_count;
            }
            entry.last_executed_tx_hash = Some(*tx_hash);
            if *tx_hash == entry.boundary.last_tx_hash {
                entry.reached_last_tx_hash = true;
            }
            entry.refresh_consistency_flags();
        }
        Some(entry.to_status())
    }

    /// Marks an existing replay boundary closed and returns its final status snapshot.
    /// Missing boundaries remain a no-op represented by `None`.
    pub fn replay_boundary_mark_closed(&self, block_n: u64) -> Option<ReplayBlockBoundaryStatus> {
        let mut guard = self.replay_boundaries.lock().expect("Poisoned lock");
        let entry = guard.get_mut(&block_n)?;
        entry.closed = true;
        Some(entry.to_status())
    }

    /// Flush all pending writes to disk. Critical for databases with WAL disabled.
    /// Must be called before shutdown to ensure data persistence.
    pub fn flush(&self) -> Result<()> {
        self.db.flush()
    }

    /// Reconciles trie durability to the backend's currently published confirmed tip.
    /// The context string is carried into diagnostics for startup or shutdown attribution.
    pub fn reconcile_confirmed_parallel_merkle_state(&self, context: &str) -> Result<()> {
        self.reconcile_confirmed_parallel_merkle_state_for_tip(self.chain_head_state().confirmed_tip, context)
    }
}

impl<D> MadaraBackend<D> {
    /// Clones a staged custom header without consuming it.
    /// Callers use this for validation before the ordered close stage.
    pub fn get_custom_header(&self, block_n: u64) -> Option<CustomHeader> {
        self.custom_headers.lock().expect("Poisoned lock").get(&block_n).cloned()
    }

    /// Removes and returns a staged custom header for one block.
    /// The operation is serialized by the custom-header mutex.
    pub fn take_custom_header(&self, block_n: u64) -> Option<CustomHeader> {
        self.custom_headers.lock().expect("Poisoned lock").remove(&block_n)
    }

    /// Removes staged custom headers at or below a confirmed block number.
    /// The return value is the number of entries cleared.
    pub fn clear_custom_headers_through(&self, block_n: u64) -> usize {
        let mut guard = self.custom_headers.lock().expect("Poisoned lock");
        let initial_len = guard.len();
        guard.retain(|stored_block_n, _| *stored_block_n > block_n);
        initial_len.saturating_sub(guard.len())
    }
}

impl<D: MadaraStorage> MadaraBackend<D> {
    /// Stages an operator-supplied header override for later block closing.
    /// Replacing the same block is allowed and logged with both configurations.
    pub fn set_custom_header(self: &Arc<Self>, custom_header: CustomHeader) -> Result<()> {
        let chain_head_state = self.chain_head_state.borrow();
        tracing::debug!(
            target: "custom_header",
            block_n = custom_header.block_n,
            timestamp = custom_header.timestamp,
            gas_prices = ?custom_header.gas_prices,
            expected_block_hash = ?custom_header.expected_block_hash,
            chain_head_state = ?*chain_head_state,
            "storing custom header"
        );
        drop(chain_head_state);

        let mut guard = self.custom_headers.lock().expect("Poisoned lock");
        if let Some(previous) = guard.insert(custom_header.block_n, custom_header.clone()) {
            tracing::debug!(
                target: "custom_header",
                block_n = custom_header.block_n,
                previous_timestamp = previous.timestamp,
                previous_gas_prices = ?previous.gas_prices,
                new_timestamp = custom_header.timestamp,
                new_gas_prices = ?custom_header.gas_prices,
                "replacing staged custom header for block"
            );
        }
        Ok(())
    }
}

impl MadaraBackend<RocksDBStorage> {
    #[cfg(any(test, feature = "testing"))]
    /// Opens an isolated temporary RocksDB backend with default test configuration.
    /// Native execution remains disabled while required compilation primitives are initialized.
    pub fn open_for_testing(chain_config: Arc<ChainConfig>) -> Arc<Self> {
        Self::open_for_testing_with_config(chain_config, Default::default())
    }

    #[cfg(any(test, feature = "testing"))]
    /// Opens an isolated temporary RocksDB backend with an explicit backend configuration.
    /// The temporary directory is retained by the backend for its full lifetime.
    pub fn open_for_testing_with_config(chain_config: Arc<ChainConfig>, config: MadaraBackendConfig) -> Arc<Self> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        let temp_dir = tempfile::TempDir::with_prefix("madara-test").unwrap();
        let db = RocksDBStorage::open(temp_dir.as_ref(), Default::default()).unwrap();
        // For tests, use default (disabled) Cairo Native config (no native execution)
        // Initialize compilation semaphore for tests (required even if native execution is disabled)
        let builder = mc_class_exec::config::NativeConfig::builder();
        let max_concurrent = builder.max_concurrent_compilations();
        mc_class_exec::init_compilation_semaphore(max_concurrent);
        let test_config = builder.build();
        let cairo_native_config = Arc::new(test_config);
        let mut backend = Self::new_and_init(db, chain_config, config, cairo_native_config).unwrap();
        backend._temp_dir = Some(temp_dir);
        Arc::new(backend)
    }

    /// Open the db.
    ///
    /// This function will:
    /// 1. Check the database version against the binary's expected version
    /// 2. Run any necessary migrations if the database is outdated
    /// 3. Create a fresh database if none exists
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The database version is newer than the binary (requires binary upgrade)
    /// - The database version is too old to migrate (requires resync)
    /// - A migration fails
    /// - The database cannot be opened
    pub fn open_rocksdb(
        base_path: &Path,
        chain_config: Arc<ChainConfig>,
        config: MadaraBackendConfig,
        rocksdb_config: RocksDBConfig,
        cairo_native_config: Arc<NativeConfig>,
    ) -> Result<Arc<Self>> {
        use crate::migration::{MigrationRunner, MigrationStatus};

        /// Database version from build-time, injected by build.rs
        const REQUIRED_DB_VERSION_STR: &str = env!("DB_VERSION");
        /// Minimum database version that can be migrated from.
        const BASE_DB_VERSION_STR: &str = env!("DB_BASE_VERSION");

        let required_version: u32 =
            REQUIRED_DB_VERSION_STR.parse().expect("DB_VERSION must be a valid u32 (checked at build time)");
        let base_version: u32 =
            BASE_DB_VERSION_STR.parse().expect("DB_BASE_VERSION must be a valid u32 (checked at build time)");

        // Create base directory if it doesn't exist
        if !base_path.exists() {
            std::fs::create_dir_all(base_path).context("Creating database directory")?;
        }

        // Check and run migrations if needed
        let migration_runner = MigrationRunner::new(base_path, required_version, base_version)
            .with_skip_backup(config.skip_migration_backup);
        let status = migration_runner.check_status().context("Checking migration status")?;

        // Handle migration status and open the database
        let db_path = base_path.join("db");
        let db = match &status {
            MigrationStatus::FreshDatabase => {
                tracing::info!("📦 Creating new database at version {}", required_version);
                // Write the version file for fresh database
                migration_runner.initialize_fresh_database().context("Initializing fresh database")?;
                RocksDBStorage::open(&db_path, rocksdb_config).context("Opening RocksDB storage")?
            }
            MigrationStatus::NoMigrationNeeded => {
                tracing::debug!("✅ Database version {} matches binary, no migration needed", required_version);
                RocksDBStorage::open(&db_path, rocksdb_config).context("Opening RocksDB storage")?
            }
            MigrationStatus::MigrationRequired { current_version, target_version, migration_count } => {
                tracing::info!(
                    "🔄 Database migration required: v{} -> v{} ({} migration(s))",
                    current_version,
                    target_version,
                    migration_count
                );
                tracing::info!("⚠️  This is a one-time operation that may take several minutes...");

                // Open the database for migration and reuse it after
                let db =
                    RocksDBStorage::open(&db_path, rocksdb_config).context("Opening RocksDB storage for migration")?;

                // Run migrations
                migration_runner.run_migrations_with_storage(&db).context("Running database migrations")?;

                // Reuse the same DB instance instead of reopening
                db
            }
            MigrationStatus::DatabaseTooOld { current_version, base_version } => {
                bail!(
                    "Database version {} is too old (minimum supported: {}). \
                    Please delete the database directory and resync from scratch.",
                    current_version,
                    base_version
                );
            }
            MigrationStatus::DatabaseNewer { db_version, binary_version } => {
                bail!(
                    "Database version {} is newer than this binary supports ({}). \
                    Please upgrade to a newer version of the binary.",
                    db_version,
                    binary_version
                );
            }
        };

        Ok(Arc::new(Self::new_and_init(db, chain_config, config, cairo_native_config)?))
    }

    /// Persists a durable parallel-Merkle checkpoint marker for one block.
    /// This facade keeps callers independent of the concrete RocksDB implementation.
    pub fn write_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<()> {
        self.db.write_parallel_merkle_checkpoint(block_n)
    }

    /// Reports whether a durable checkpoint marker exists for one block.
    /// Storage failures are preserved rather than treated as absence.
    pub fn has_parallel_merkle_checkpoint(&self, block_n: u64) -> Result<bool> {
        self.db.has_parallel_merkle_checkpoint(block_n)
    }

    /// Returns the latest durable checkpoint block recorded by parallel Merkle.
    /// An empty database returns `None` without manufacturing a genesis checkpoint.
    pub fn get_parallel_merkle_latest_checkpoint(&self) -> Result<Option<u64>> {
        self.db.get_parallel_merkle_latest_checkpoint()
    }
}
