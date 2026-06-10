use crate::{
    import::{BlockImportError, BlockImporter},
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
};
use anyhow::Context;
use mc_db::MadaraBackend;
use mc_gateway_client::{BlockId, GatewayProvider};
use mp_class::{
    ClassInfo, ClassInfoWithHash, CompiledSierra, ConvertedClass, LegacyClassInfo, SierraClassInfo, MISSED_CLASS_HASHES,
};
use mp_state_update::DeclaredClassCompiledClass;
use mp_utils::AbortOnDrop;
use starknet_api::core::ChainId;
use starknet_core::types::Felt;
use std::{collections::HashMap, ops::Range, sync::Arc};

/// Some historical mainnet gateway state updates omitted legacy class declarations.
/// Keep an explicit repair table keyed by block number.
fn fixup_missed_mainnet_classes(block_n: u64, classes_from_state_diff: &mut HashMap<Felt, DeclaredClassCompiledClass>) {
    if let Some(class_hashes) = MISSED_CLASS_HASHES.get(&block_n) {
        classes_from_state_diff
            .extend(class_hashes.iter().copied().map(|hash| (hash, DeclaredClassCompiledClass::Legacy)))
    }
}

fn should_fixup_missed_mainnet_classes(chain_id: ChainId) -> bool {
    chain_id == ChainId::Mainnet
}

/// Fetches class definitions from the gateway and creates ClassInfo structures.
///
/// # Arguments
/// * `uses_blake_hash` - If true, the compiled_class_hash from state diff is a BLAKE hash (v0.14.1+)
///   and should be stored in `compiled_class_hash_v2`. Otherwise it's a Poseidon
///   hash and goes in `compiled_class_hash`.
pub(crate) async fn get_classes(
    client: &Arc<GatewayProvider>,
    block_id: BlockId,
    classes: &HashMap<Felt, DeclaredClassCompiledClass>,
    uses_blake_hash: bool,
) -> anyhow::Result<Vec<ClassInfoWithHash>> {
    futures::future::try_join_all(classes.iter().map(move |(&class_hash, &compiled_class_hash)| {
        let block_id = block_id.clone();
        let client = client.clone();
        async move {
            let class = client
                .clone()
                .get_class_by_hash(class_hash, block_id.clone())
                .await
                .with_context(|| format!("Getting class_hash={class_hash:#x} with block_id={block_id:?}"))?;

            let class_info = match &class {
                mp_class::ContractClass::Sierra(class) => {
                    let DeclaredClassCompiledClass::Sierra(compiled_class_hash) = compiled_class_hash else {
                        anyhow::bail!("Expected a Sierra class, found a Legacy class")
                    };
                    // For v0.14.1+ blocks, newly declared classes use BLAKE hash (v2).
                    // For pre-v0.14.1 blocks, classes use Poseidon hash (v1).
                    let (v1_hash, v2_hash) = if uses_blake_hash {
                        (None, Some(compiled_class_hash))
                    } else {
                        (Some(compiled_class_hash), None)
                    };
                    ClassInfo::Sierra(SierraClassInfo {
                        contract_class: class.clone(),
                        compiled_class_hash: v1_hash,
                        compiled_class_hash_v2: v2_hash,
                    })
                }
                mp_class::ContractClass::Legacy(class) => {
                    if compiled_class_hash != DeclaredClassCompiledClass::Legacy {
                        anyhow::bail!("Expected a Legacy class, found a Sierra class")
                    }
                    ClassInfo::Legacy(LegacyClassInfo { contract_class: class.clone() })
                }
            };

            Ok(ClassInfoWithHash { class_info, class_hash })
        }
    }))
    .await
}

/// Verifies and compiles the declared classes of a block, falling back to the canonical CASM
/// served by the feeder gateway when local Sierra-to-CASM compilation fails.
///
/// Some early-2023 historical Sierra classes (e.g. mainnet class
/// `0x78d552edf8d22e566b050c9158d7f770b55021c36a9f5a98170ff8fcabc9e10`, declared around block
/// 80000) were compiled with ancient Cairo toolchains and can no longer be recompiled by the
/// modern `cairo-lang-starknet-classes` crates. Reference nodes (Juno, Pathfinder) handle these
/// classes by using the canonical CASM from the feeder gateway instead of recompiling locally.
///
/// Trust model: in gateway sync mode the feeder gateway is already the trust root for blocks,
/// state diffs and class definitions, so falling back to its CASM does not extend trust. We still
/// verify what is verifiable:
/// - The fallback only engages when local compilation *errors* (never on a plain hash mismatch),
///   so it cannot mask compiler regressions on classes that still compile.
/// - The fetched CASM is re-hashed; the hash check in
///   [`crate::import::BlockImporterCtx::verify_compile_classes`] then applies as usual: a match
///   is accepted, a mismatch is only tolerated under the pre-existing historical
///   compiled_class_hash tolerance (pre-v0.14.1 Poseidon classes) and is rejected for modern
///   (v0.14.1+, BLAKE) classes.
pub(crate) async fn verify_compile_classes_with_gateway_casm_fallback(
    importer: &BlockImporter,
    client: &Arc<GatewayProvider>,
    block_n: u64,
    declared_classes: Vec<ClassInfoWithHash>,
    check_against: &HashMap<Felt, DeclaredClassCompiledClass>,
) -> Result<Vec<ConvertedClass>, BlockImportError> {
    let mut gateway_casm_fallback: HashMap<Felt, CompiledSierra> = HashMap::new();
    // Each iteration either succeeds, fails for good, or adds a fallback CASM for a class hash
    // that did not have one yet, so this loop runs at most `len + 1` times.
    for _ in 0..=check_against.len() {
        let declared_classes_ = declared_classes.clone();
        let check_against_ = check_against.clone();
        let gateway_casm_fallback_ = gateway_casm_fallback.clone();
        let res = importer
            .run_in_rayon_pool(move |importer| {
                importer.verify_compile_classes(
                    Some(block_n),
                    declared_classes_,
                    &check_against_,
                    &gateway_casm_fallback_,
                )
            })
            .await;
        match res {
            Err(BlockImportError::CompilationClassError { class_hash, error })
                if !gateway_casm_fallback.contains_key(&class_hash) =>
            {
                tracing::warn!(
                    class_hash = %format!("{class_hash:#x}"),
                    block_n,
                    error = %error,
                    "Local CASM compilation failed for class; falling back to the compiled class \
                     (CASM) served by the feeder gateway",
                );
                let compiled = client
                    .get_compiled_class_by_class_hash(class_hash, BlockId::Number(block_n))
                    .await
                    .map_err(|fetch_error| {
                        BlockImportError::Internal(anyhow::Error::new(fetch_error).context(format!(
                            "Fetching compiled class (CASM) from the gateway for class_hash={class_hash:#x} \
                             block_n={block_n} after local compilation failed: {error}"
                        )))
                    })?;
                gateway_casm_fallback.insert(class_hash, compiled);
            }
            res => return res,
        }
    }
    Err(BlockImportError::Internal(anyhow::anyhow!(
        "Gateway CASM fallback did not converge for block_n={block_n} (this is a bug)"
    )))
}

pub type ClassesSync = PipelineController<ClassesSyncSteps>;
pub fn classes_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    starting_block: u64,
    parallelization: usize,
    batch_size: usize,
) -> ClassesSync {
    PipelineController::new(ClassesSyncSteps { backend, importer, client }, parallelization, batch_size, starting_block)
}

pub struct ClassesSyncSteps {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
}
impl PipelineSteps for ClassesSyncSteps {
    type InputItem = HashMap<Felt, DeclaredClassCompiledClass>;
    type SequentialStepInput = Vec<Vec<ConvertedClass>>;
    type Output = ();

    async fn parallel_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        mut input: Vec<Self::InputItem>,
    ) -> anyhow::Result<Self::SequentialStepInput> {
        if should_fixup_missed_mainnet_classes(self.backend.chain_config().chain_id.clone()) {
            block_range
                .clone()
                .zip(input.iter_mut())
                .for_each(|(block_n, classes)| fixup_missed_mainnet_classes(block_n, classes));
        }
        if input.iter().all(|i| i.is_empty()) {
            return Ok(vec![]);
        }

        AbortOnDrop::spawn(async move {
            tracing::debug!("Gateway classes parallel step: {block_range:?}");
            let mut out = vec![];
            for (block_n, classes) in block_range.zip(input) {
                // Get the block's protocol version from the already-saved header
                // to determine if we should use BLAKE hash (v0.14.1+) or Poseidon hash
                let uses_blake_hash = self
                    .backend
                    .block_view_on_confirmed(block_n)
                    .and_then(|view| view.get_block_info().ok())
                    .map(|info| info.header.protocol_version.uses_blake_compiled_class_hash())
                    .unwrap_or(false);

                let declared_classes =
                    get_classes(&self.client, BlockId::Number(block_n), &classes, uses_blake_hash).await?;

                let ret = verify_compile_classes_with_gateway_casm_fallback(
                    &self.importer,
                    &self.client,
                    block_n,
                    declared_classes,
                    &classes,
                )
                .await
                .with_context(|| format!("Verifying and compiling classes for block_n={block_n:?}"))?;

                out.push(ret);
            }
            Ok(out)
        })
        .await
    }

    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Self::SequentialStepInput,
        _target_block: Option<u64>,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>> {
        if input.iter().all(|i| i.is_empty()) {
            return Ok(ApplyOutcome::Success(()));
        }
        tracing::debug!("Gateway classes sequential step: {block_range:?}");
        // Save classes in sequential step, because some chains have duplicate class declarations, and we want to be sure
        // we always record the earliest block_n
        let block_range_ = block_range.clone();
        self.importer
            .run_in_rayon_pool(move |importer| {
                for (block_n, input) in block_range_.zip(input) {
                    importer.save_classes(block_n, input)?;
                }
                anyhow::Ok(())
            })
            .await
            .with_context(|| format!("Saving classes for block_range={block_range:?}"))?;
        Ok(ApplyOutcome::Success(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{BlockImportError, BlockImporter, BlockValidationConfig};
    use crate::tests::gateway_mock::GatewayMock;
    use assert_matches::assert_matches;
    use mp_block::{BlockHeaderWithSignatures, Header};
    use mp_chain_config::{ChainConfig, StarknetVersion};
    use mp_class::compile::CompiledClassHashes;
    use mp_class::FlattenedSierraClass;
    use starknet_api::felt;
    use std::collections::HashMap;

    const CLASS_HASH: Felt =
        Felt::from_hex_unchecked("0x78d552edf8d22e566b050c9158d7f770b55021c36a9f5a98170ff8fcabc9e10");

    struct FallbackFixture {
        backend: Arc<mc_db::MadaraBackend>,
        importer: BlockImporter,
        gateway_mock: GatewayMock,
        /// A Sierra class that compiles fine locally.
        good_sierra: Arc<FlattenedSierraClass>,
        /// A Sierra class that fails local compilation (truncated program), standing in for the
        /// early-2023 historical classes that modern compilers can no longer compile.
        broken_sierra: Arc<FlattenedSierraClass>,
        /// JSON-serialized canonical CASM of `good_sierra`, as the feeder gateway would serve it.
        casm_json: String,
        /// Hashes of `casm_json`.
        casm_hashes: CompiledClassHashes,
    }

    fn fallback_fixture() -> FallbackFixture {
        let mut json: serde_json::Value = serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
        let abi_string = serde_json::to_string(&json["abi"]).unwrap();
        json["abi"] = serde_json::Value::String(abi_string);
        let good_sierra: FlattenedSierraClass = serde_json::from_value(json).unwrap();

        let (_poseidon_hash, casm) = good_sierra.compile_to_casm().unwrap();
        let casm_json = serde_json::to_string(&casm).unwrap();
        let casm_hashes = CompiledClassHashes::from_casm(casm).unwrap();

        let mut broken_sierra = good_sierra.clone();
        // Keep the version header felts so version parsing succeeds, but make the program
        // impossible to compile.
        broken_sierra.sierra_program.truncate(3);
        assert!(broken_sierra.compile_to_casm().is_err(), "the broken fixture class must fail to compile");

        let backend = mc_db::MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        // `trust_class_hashes` only skips the (expensive) sierra class hash recomputation: the
        // fixture class hash is arbitrary. All other checks stay enabled.
        let importer = BlockImporter::new(
            backend.clone(),
            BlockValidationConfig { trust_class_hashes: true, ..Default::default() },
        );

        FallbackFixture {
            backend,
            importer,
            gateway_mock: GatewayMock::new(),
            good_sierra: Arc::new(good_sierra),
            broken_sierra: Arc::new(broken_sierra),
            casm_json,
            casm_hashes,
        }
    }

    fn declared_sierra_class(
        contract_class: Arc<FlattenedSierraClass>,
        compiled_class_hash: Felt,
    ) -> ClassInfoWithHash {
        ClassInfoWithHash {
            class_hash: CLASS_HASH,
            class_info: ClassInfo::Sierra(SierraClassInfo {
                contract_class,
                compiled_class_hash: Some(compiled_class_hash),
                compiled_class_hash_v2: None,
            }),
        }
    }

    /// Compile failure -> CASM fetched from the gateway -> recomputed hash matches the declared
    /// compiled_class_hash -> accepted.
    #[tokio::test]
    async fn casm_fallback_verified_accept() {
        let fixture = fallback_fixture();
        let declared_hash = fixture.casm_hashes.poseidon_hash;
        let casm_mock =
            fixture.gateway_mock.mock_compiled_class_from_json(format!("{CLASS_HASH:#x}"), fixture.casm_json.clone());

        let check_against = HashMap::from([(CLASS_HASH, DeclaredClassCompiledClass::Sierra(declared_hash))]);
        let converted = verify_compile_classes_with_gateway_casm_fallback(
            &fixture.importer,
            &fixture.gateway_mock.client(),
            0,
            vec![declared_sierra_class(fixture.broken_sierra.clone(), declared_hash)],
            &check_against,
        )
        .await
        .unwrap();

        assert_eq!(casm_mock.hits(), 1);
        assert_eq!(converted.len(), 1);
        let sierra = converted[0].as_sierra().unwrap();
        assert_eq!(sierra.class_hash, CLASS_HASH);
        assert_eq!(sierra.info.compiled_class_hash, Some(declared_hash));
        // The stored CASM is the gateway-provided one.
        assert_eq!(
            CompiledClassHashes::from_compiled_sierra(&sierra.compiled).unwrap().poseidon_hash,
            fixture.casm_hashes.poseidon_hash
        );
    }

    /// Compile failure -> CASM fetched from the gateway -> recomputed hash does NOT match, but the
    /// class is historical (pre-v0.14.1, Poseidon) -> accepted with the gateway-declared hash,
    /// mirroring the pre-existing "Retaining gateway-provided historical compiled_class_hash"
    /// tolerance for local compiles.
    #[tokio::test]
    async fn casm_fallback_historical_mismatch_accepted() {
        let fixture = fallback_fixture();
        let declared_hash = felt!("0xdeadbeef"); // does not match the fetched CASM
        fixture.gateway_mock.mock_compiled_class_from_json(format!("{CLASS_HASH:#x}"), fixture.casm_json.clone());

        let check_against = HashMap::from([(CLASS_HASH, DeclaredClassCompiledClass::Sierra(declared_hash))]);
        let converted = verify_compile_classes_with_gateway_casm_fallback(
            &fixture.importer,
            &fixture.gateway_mock.client(),
            0,
            vec![declared_sierra_class(fixture.broken_sierra.clone(), declared_hash)],
            &check_against,
        )
        .await
        .unwrap();

        assert_eq!(converted.len(), 1);
        let sierra = converted[0].as_sierra().unwrap();
        // The gateway-declared (state diff) hash is retained, not the recomputed one.
        assert_eq!(sierra.info.compiled_class_hash, Some(declared_hash));
    }

    /// Compile failure -> CASM fetched from the gateway -> recomputed hash does NOT match and the
    /// class is NOT historical (v0.14.1+, BLAKE) -> rejected: the historical tolerance must not
    /// mask mismatches on modern classes.
    #[tokio::test]
    async fn casm_fallback_non_historical_mismatch_rejected() {
        let fixture = fallback_fixture();
        let declared_hash = felt!("0xdeadbeef"); // does not match the fetched CASM
        fixture.gateway_mock.mock_compiled_class_from_json(format!("{CLASS_HASH:#x}"), fixture.casm_json.clone());

        // Store a v0.14.1 header for block 0 so the class is treated as a modern (BLAKE) class.
        fixture
            .backend
            .write_access()
            .write_header(BlockHeaderWithSignatures {
                block_hash: felt!("0x123"),
                consensus_signatures: vec![],
                header: Header { block_number: 0, protocol_version: StarknetVersion::V0_14_1, ..Default::default() },
            })
            .unwrap();

        let check_against = HashMap::from([(CLASS_HASH, DeclaredClassCompiledClass::Sierra(declared_hash))]);
        let result = verify_compile_classes_with_gateway_casm_fallback(
            &fixture.importer,
            &fixture.gateway_mock.client(),
            0,
            vec![declared_sierra_class(fixture.broken_sierra.clone(), declared_hash)],
            &check_against,
        )
        .await;

        assert_matches!(
            result,
            Err(BlockImportError::CompiledClassHash { class_hash, got, expected }) => {
                assert_eq!(class_hash, CLASS_HASH);
                assert_eq!(got, declared_hash);
                assert_eq!(expected, fixture.casm_hashes.blake_hash);
            }
        );
    }

    /// Compile failure -> the gateway CASM fetch itself fails -> the error is propagated and sync
    /// fails as it does today.
    #[tokio::test]
    async fn casm_fallback_fetch_error_propagates() {
        let fixture = fallback_fixture();
        let declared_hash = fixture.casm_hashes.poseidon_hash;
        fixture.gateway_mock.mock_compiled_class_not_found(format!("{CLASS_HASH:#x}"));

        let check_against = HashMap::from([(CLASS_HASH, DeclaredClassCompiledClass::Sierra(declared_hash))]);
        let result = verify_compile_classes_with_gateway_casm_fallback(
            &fixture.importer,
            &fixture.gateway_mock.client(),
            0,
            vec![declared_sierra_class(fixture.broken_sierra.clone(), declared_hash)],
            &check_against,
        )
        .await;

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("Fetching compiled class (CASM) from the gateway"), "{err}");
        assert!(err.contains(&format!("{CLASS_HASH:#x}")), "{err}");
    }

    /// The fallback must only engage on local compilation *failure*: a class that compiles
    /// locally never triggers a gateway CASM fetch, even if its hash mismatches (which keeps the
    /// pre-existing tolerance behavior and cannot mask local compiler regressions).
    #[tokio::test]
    async fn casm_fallback_not_engaged_when_local_compile_succeeds() {
        let fixture = fallback_fixture();
        let declared_hash = fixture.casm_hashes.poseidon_hash;
        let casm_mock =
            fixture.gateway_mock.mock_compiled_class_from_json(format!("{CLASS_HASH:#x}"), fixture.casm_json.clone());

        let check_against = HashMap::from([(CLASS_HASH, DeclaredClassCompiledClass::Sierra(declared_hash))]);
        let converted = verify_compile_classes_with_gateway_casm_fallback(
            &fixture.importer,
            &fixture.gateway_mock.client(),
            0,
            vec![declared_sierra_class(fixture.good_sierra.clone(), declared_hash)],
            &check_against,
        )
        .await
        .unwrap();

        assert_eq!(casm_mock.hits(), 0);
        assert_eq!(converted.len(), 1);
    }

    #[test]
    fn fixup_missed_mainnet_classes_leaves_unknown_blocks_unchanged() {
        let existing = Felt::from_hex_unchecked("0x123");
        let mut classes = HashMap::from([(existing, DeclaredClassCompiledClass::Legacy)]);

        fixup_missed_mainnet_classes(2597, &mut classes);

        assert_eq!(classes, HashMap::from([(existing, DeclaredClassCompiledClass::Legacy)]));
    }

    #[test]
    fn fixup_missed_mainnet_classes_adds_known_post_2597_repairs() {
        let existing = Felt::from_hex_unchecked("0x123");
        let repaired = Felt::from_hex_unchecked("0x26fe8ea36ec7703569cfe4693b05102940bf122647c4dbf0abc0bb919ce27bd");
        let mut classes = HashMap::from([(existing, DeclaredClassCompiledClass::Legacy)]);

        fixup_missed_mainnet_classes(5982, &mut classes);

        assert_eq!(classes.get(&existing), Some(&DeclaredClassCompiledClass::Legacy));
        assert_eq!(classes.get(&repaired), Some(&DeclaredClassCompiledClass::Legacy));
    }

    #[test]
    fn missing_class_repair_is_only_enabled_on_mainnet() {
        assert!(should_fixup_missed_mainnet_classes(ChainId::Mainnet));
        assert!(!should_fixup_missed_mainnet_classes(ChainId::Sepolia));
    }
}
