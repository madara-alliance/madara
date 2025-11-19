use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
};
use anyhow::Context;
use mc_db::MadaraBackend;
use mc_gateway_client::{BlockId, GatewayProvider};
use mp_class::{ClassInfo, ClassInfoWithHash, ConvertedClass, LegacyClassInfo, SierraClassInfo, MISSED_CLASS_HASHES};
use mp_state_update::DeclaredClassCompiledClass;
use mp_utils::AbortOnDrop;
use starknet_api::core::ChainId;
use starknet_core::types::Felt;
use std::{collections::HashMap, ops::Range, sync::Arc};

/// for blocks before 2597 on mainnet new classes are not declared in the state update
/// https://github.com/madara-alliance/madara/issues/233
fn fixup_missed_mainnet_classes(block_n: u64, classes_from_state_diff: &mut HashMap<Felt, DeclaredClassCompiledClass>) {
    if block_n < 2597 {
        classes_from_state_diff.extend(
            MISSED_CLASS_HASHES
                .get(&block_n)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|hash| (hash, DeclaredClassCompiledClass::Legacy)),
        )
    }
}

pub(crate) async fn get_classes(
    client: &Arc<GatewayProvider>,
    block_id: BlockId,
    classes: &HashMap<Felt, DeclaredClassCompiledClass>,
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
                    ClassInfo::Sierra(SierraClassInfo { contract_class: class.clone(), compiled_class_hash })
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
        if self.backend.chain_config().chain_id == ChainId::Mainnet {
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
                let declared_classes = get_classes(&self.client, BlockId::Number(block_n), &classes).await?;

                let ret = self
                    .importer
                    .run_in_rayon_pool(move |importer| {
                        importer.verify_compile_classes(Some(block_n), declared_classes, &classes)
                    })
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
