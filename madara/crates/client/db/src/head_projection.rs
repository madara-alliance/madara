use super::*;

impl<D: MadaraStorageRead> MadaraBackend<D> {
    /// Increments the invariant metric and logs one rejected head-projection state.
    /// Callers still return their original error so diagnostics do not alter control flow.
    fn register_projection_violation(message: String) {
        metrics().head_projection_violation_count.add(1, &[]);
        #[cfg(test)]
        metrics().head_projection_violation_count_test.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::error!(target: "db::chain_head_projection", "{message}");
    }

    /// Loads the durable preconfirmed block named by a projected tip.
    /// Missing header/content pairs are reported as projection corruption.
    fn load_preconfirmed_block_for_tip(
        &self,
        block_n: u64,
        stored_tip: &StorageHeadProjection,
    ) -> Result<Arc<PreconfirmedBlock>> {
        if let StorageHeadProjection::Preconfirmed { header, content } = stored_tip {
            if header.block_number == block_n {
                return Ok(Arc::new(PreconfirmedBlock::new_with_content(
                    header.clone(),
                    content.clone(),
                    /* candidates */ [],
                )));
            }
        }

        let (header, content) = self
            .db
            .get_preconfirmed_block_data(block_n)?
            .with_context(|| format!("Expected persisted preconfirmed block data for block #{block_n}"))?;
        Ok(Arc::new(PreconfirmedBlock::new_with_content(header, content, /* candidates */ [])))
    }

    /// Reconstructs runtime confirmed/internal/external heads from durable projection data.
    /// The returned block is the persisted internal preconfirmed frontier, when present.
    pub(super) fn build_runtime_head_projection(
        &self,
        stored_tip: StorageHeadProjection,
    ) -> Result<(ChainHeadState, Option<Arc<PreconfirmedBlock>>)> {
        let mut chain_head_state = ChainHeadState::from_head_projection(&stored_tip);
        let latest_preconfirmed_header_block_n = self.db.get_latest_preconfirmed_header_block_n()?;

        chain_head_state.internal_preconfirmed_tip = match (
            chain_head_state.external_preconfirmed_tip,
            latest_preconfirmed_header_block_n,
        ) {
            (None, None) => None,
            (Some(external_tip), None) => Some(external_tip),
            (Some(external_tip), Some(latest_header_tip)) => {
                ensure!(
                    latest_header_tip >= external_tip,
                    "Latest persisted preconfirmed header tip ({latest_header_tip}) is behind external preconfirmed tip ({external_tip}). [stored_tip={stored_tip:?}]"
                );
                Some(latest_header_tip)
            }
            (None, Some(latest_header_tip)) => {
                bail!(
                    "Found persisted preconfirmed header tip ({latest_header_tip}) while head projection has no external preconfirmed tip. [stored_tip={stored_tip:?}]"
                );
            }
        };

        let runtime_preconfirmed = if let Some(internal_tip) = chain_head_state.internal_preconfirmed_tip {
            Some(self.load_preconfirmed_block_for_tip(internal_tip, &stored_tip)?)
        } else {
            None
        };

        Ok((chain_head_state, runtime_preconfirmed))
    }

    /// Verifies runtime preconfirmed blocks agree with the projected tips and confirmed floor.
    /// Gaps and blocks ahead of the internal frontier are rejected before publication.
    pub(super) fn ensure_runtime_preconfirmed_alignment(
        chain_head_state: ChainHeadState,
        preconfirmed: &RuntimePreconfirmedBlocks,
    ) -> Result<()> {
        match chain_head_state.internal_preconfirmed_tip {
            None if preconfirmed.is_empty() => Ok(()),
            Some(expected) if preconfirmed.contains_key(&expected) => {
                if let Some(confirmed_tip) = chain_head_state.confirmed_tip {
                    ensure!(
                        preconfirmed.keys().all(|block_n| *block_n > confirmed_tip),
                        "Runtime preconfirmed blocks must be strictly above confirmed_tip {}. [head={chain_head_state:?}, runtime_blocks={:?}]",
                        confirmed_tip,
                        preconfirmed.keys().copied().collect::<Vec<_>>()
                    );
                }
                ensure!(
                    preconfirmed.keys().all(|block_n| *block_n <= expected),
                    "Runtime preconfirmed blocks must not be ahead of internal_preconfirmed_tip {}. [head={chain_head_state:?}, runtime_blocks={:?}]",
                    expected,
                    preconfirmed.keys().copied().collect::<Vec<_>>()
                );
                Ok(())
            }
            Some(expected) => {
                let message = format!(
                    "Runtime preconfirmed block is missing while head expects internal preconfirmed tip {}. [head={chain_head_state:?}, runtime_blocks={:?}]",
                    expected,
                    preconfirmed.keys().copied().collect::<Vec<_>>()
                );
                Self::register_projection_violation(message.clone());
                bail!("{message}");
            }
            None => {
                let message = format!(
                    "Runtime preconfirmed blocks {:?} exist while head has no internal preconfirmed tip. [head={chain_head_state:?}]",
                    preconfirmed.keys().copied().collect::<Vec<_>>()
                );
                Self::register_projection_violation(message.clone());
                bail!("{message}");
            }
        }
    }

    /// Ensure projected storage tip never gets ahead of canonical chain head state.
    ///
    /// Stale/lower confirmed values are allowed (lagging projection), but future/incompatible values are rejected.
    pub(crate) fn ensure_tip_not_ahead_of_head_state(
        chain_head_state: ChainHeadState,
        projected_storage_tip: &StorageHeadProjection,
    ) -> Result<()> {
        match projected_storage_tip {
            StorageHeadProjection::Empty => Ok(()),
            StorageHeadProjection::Confirmed(block_n) => {
                let is_allowed = chain_head_state.confirmed_tip.is_some_and(|confirmed| *block_n <= confirmed);
                if !is_allowed {
                    let message = format!(
                        "Projected storage tip confirmed block_n={} is ahead of canonical head confirmed_tip={:?}. [head={chain_head_state:?}, tip={projected_storage_tip:?}]",
                        block_n,
                        chain_head_state.confirmed_tip
                    );
                    Self::register_projection_violation(message.clone());
                    bail!("{message}");
                }
                Ok(())
            }
            StorageHeadProjection::Preconfirmed { header, .. } => {
                let expected = chain_head_state.external_preconfirmed_tip;
                if expected != Some(header.block_number) {
                    let message = format!(
                        "Projected storage tip preconfirmed block_n={} is incompatible with canonical head external_preconfirmed_tip={:?}. [head={chain_head_state:?}, tip={projected_storage_tip:?}]",
                        header.block_number,
                        expected
                    );
                    Self::register_projection_violation(message.clone());
                    bail!("{message}");
                }
                Ok(())
            }
        }
    }

    /// Replaces runtime preconfirmed blocks and publishes their coherent head state.
    /// Validation completes before either observable structure is changed.
    pub(super) fn publish_head_projection(
        &self,
        chain_head_state: ChainHeadState,
        preconfirmed: Option<Arc<PreconfirmedBlock>>,
    ) -> Result<()> {
        chain_head_state.validate_cross_field_invariants().map_err(|err| {
            let message = err.to_string();
            Self::register_projection_violation(message.clone());
            anyhow::anyhow!(message)
        })?;

        let previous_head_state = *self.chain_head_state.borrow();
        let current_runtime_preconfirmed = self.preconfirmed_block_runtime.read().expect("Poisoned lock").clone();
        let previous_runtime_preconfirmed_block_n = runtime_preconfirmed_tip_block_n(&current_runtime_preconfirmed);

        let mut next_runtime_preconfirmed = current_runtime_preconfirmed;
        if let Some(block) = preconfirmed {
            next_runtime_preconfirmed.insert(block.header.block_number, block);
        }
        prune_runtime_preconfirmed_blocks(&mut next_runtime_preconfirmed, chain_head_state);

        Self::ensure_runtime_preconfirmed_alignment(chain_head_state, &next_runtime_preconfirmed)?;
        let projected_tip = storage_tip_from_head_projection(
            chain_head_state,
            chain_head_state
                .external_preconfirmed_tip
                .and_then(|block_n| runtime_preconfirmed_block(&next_runtime_preconfirmed, block_n)),
        );
        Self::ensure_tip_not_ahead_of_head_state(chain_head_state, &projected_tip)?;

        let next_runtime_preconfirmed_block_n = runtime_preconfirmed_tip_block_n(&next_runtime_preconfirmed);
        let transition = classify_chain_head_transition(previous_head_state, chain_head_state);

        tracing::info!(
            target: "db::chain_head_projection",
            transition,
            previous_confirmed_tip = ?previous_head_state.confirmed_tip,
            previous_external_preconfirmed_tip = ?previous_head_state.external_preconfirmed_tip,
            previous_internal_preconfirmed_tip = ?previous_head_state.internal_preconfirmed_tip,
            next_confirmed_tip = ?chain_head_state.confirmed_tip,
            next_external_preconfirmed_tip = ?chain_head_state.external_preconfirmed_tip,
            next_internal_preconfirmed_tip = ?chain_head_state.internal_preconfirmed_tip,
            previous_runtime_preconfirmed_block_n = ?previous_runtime_preconfirmed_block_n,
            next_runtime_preconfirmed_block_n = ?next_runtime_preconfirmed_block_n,
            "chain_head_state_updated"
        );
        // Publish runtime preconfirmed first, then canonical head state.
        // This narrows the window where readers can observe a new head with stale runtime preconfirmed.
        *self.preconfirmed_block_runtime.write().expect("Poisoned lock") = next_runtime_preconfirmed;
        self.chain_head_state.send_replace(chain_head_state);

        Ok(())
    }

    /// Rebuilds the runtime head while the caller holds `head_projection_write_lock`.
    /// Durable rows are validated before the reconstructed state is published.
    pub(super) fn refresh_head_projection_from_db_locked(&self) -> Result<()> {
        let (chain_head_state, preconfirmed) = self.build_runtime_head_projection(self.db.get_head_projection()?)?;
        self.publish_head_projection(chain_head_state, preconfirmed)
    }

    /// Refreshes the in-memory head projection from its persisted representation.
    ///
    /// This is the canonical refresh path used by RPC and sync compatibility callsites after
    /// destructive DB operations. Refresh is serialized with live head transitions.
    pub fn refresh_head_projection_from_db(&self) -> Result<()> {
        let _projection_guard = self.head_projection_write_lock.lock().expect("Poisoned head projection lock");
        self.refresh_head_projection_from_db_locked()
    }

    /// Returns the authoritative confirmed block number currently published in memory.
    /// Empty chains return `None`.
    pub fn latest_confirmed_block_n(&self) -> Option<u64> {
        self.chain_head_state.borrow().confirmed_tip
    }
    /// Latest block_n, which may be the pre-confirmed block.
    /// External visibility is used, so internal runahead is intentionally excluded.
    pub fn latest_block_n(&self) -> Option<u64> {
        let head = self.chain_head_state.borrow();
        head.external_preconfirmed_tip.or(head.confirmed_tip)
    }
    /// Reports whether an externally visible preconfirmed block is currently projected.
    /// Internal-only runahead does not satisfy this public-facing predicate.
    pub fn has_preconfirmed_block(&self) -> bool {
        self.chain_head_state.borrow().external_preconfirmed_tip.is_some()
    }
    /// Returns the most recent L2 block known to be confirmed on L1.
    /// The value is distributed through a watch channel for subscribers.
    pub fn latest_l1_confirmed_block_n(&self) -> Option<u64> {
        *self.latest_l1_confirmed.borrow()
    }

    /// Returns the newest runtime preconfirmed block, including internal runahead.
    /// The shared block is cloned without retaining the runtime-map read lock.
    pub(crate) fn internal_preconfirmed_block(&self) -> Option<Arc<PreconfirmedBlock>> {
        let expected_preconfirmed = self.chain_head_state.borrow().internal_preconfirmed_tip?;
        runtime_preconfirmed_block(
            &self.preconfirmed_block_runtime.read().expect("Poisoned lock"),
            expected_preconfirmed,
        )
    }

    /// Returns the externally visible preconfirmed block from runtime or durable fallback data.
    /// Internal-only runahead blocks remain hidden from this accessor.
    pub fn preconfirmed_block(&self) -> Option<Arc<PreconfirmedBlock>> {
        let expected_preconfirmed = self.chain_head_state.borrow().external_preconfirmed_tip?;
        if let Some(runtime) = runtime_preconfirmed_block(
            &self.preconfirmed_block_runtime.read().expect("Poisoned lock"),
            expected_preconfirmed,
        ) {
            return Some(runtime);
        }

        match self.db.get_preconfirmed_block_data(expected_preconfirmed) {
            Ok(Some((header, content))) => {
                Some(Arc::new(PreconfirmedBlock::new_with_content(header, content, /* candidates */ [])))
            }
            Ok(None) => {
                tracing::warn!(
                    "Missing external preconfirmed block #{expected_preconfirmed} while head projection expects it"
                );
                None
            }
            Err(err) => {
                tracing::warn!("Failed to load external preconfirmed block #{expected_preconfirmed} from db: {err:#}");
                None
            }
        }
    }

    /// Returns a copy of the coherent confirmed, external, and internal head projection.
    /// Reading the watch sender avoids acquiring the preconfirmed runtime-map lock.
    pub fn chain_head_state(&self) -> ChainHeadState {
        *self.chain_head_state.borrow()
    }

    /// Get the latest block_n that was in the db when this backend instance was initialized.
    /// This startup value is distinct from the head advanced during the current process.
    pub fn get_starting_block(&self) -> Option<u64> {
        self.starting_block
    }

    /// Borrows the immutable chain configuration shared by backend consumers.
    /// Its lifetime matches the backend instance.
    pub fn chain_config(&self) -> &Arc<ChainConfig> {
        &self.chain_config
    }

    /// Borrows the configured execution read-cache limits.
    /// Executors use this to construct cache layers consistently.
    pub fn execution_read_cache_config(&self) -> &ExecutionReadCacheConfig {
        &self.config.execution_read_cache
    }

    /// Reports whether executed preconfirmed content is persisted for crash recovery.
    /// Disabled persistence provides runtime visibility only.
    pub fn saves_preconfirmed_blocks(&self) -> bool {
        self.config.save_preconfirmed
    }

    /// Get the runtime execution configuration from the database.
    /// Missing configuration remains `None` so startup can apply current defaults.
    pub fn get_runtime_exec_config(&self) -> Result<Option<mp_chain_config::RuntimeExecutionConfig>> {
        self.db.get_runtime_exec_config(&self.chain_config)
    }
}

impl<D: MadaraStorage> MadaraBackend<D> {
    /// Clear any saved runtime execution configuration from the database.
    /// Full-node startup uses this to avoid inheriting sequencer execution policy.
    pub fn clear_runtime_exec_config(&self) -> Result<()> {
        self.db.clear_runtime_exec_config()
    }
}
