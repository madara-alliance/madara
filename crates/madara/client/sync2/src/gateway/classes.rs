use crate::{
    import::BlockImporter,
    pipeline::{ApplyOutcome, PipelineController, PipelineSteps},
    util::AbortOnDrop,
};
use anyhow::Context;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mp_block::BlockId;
use mp_class::{ClassInfo, ClassInfoWithHash, LegacyClassInfo, SierraClassInfo};
use mp_state_update::DeclaredClassCompiledClass;
use starknet_core::types::Felt;
use std::{collections::HashMap, ops::Range, sync::Arc};

pub type ClassesSync = PipelineController<ClassesSyncSteps>;
pub fn classes_pipeline(
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
    parallelization: usize,
    batch_size: usize,
) -> ClassesSync {
    PipelineController::new(ClassesSyncSteps { backend, importer, client }, parallelization, batch_size)
}

pub struct ClassesSyncSteps {
    backend: Arc<MadaraBackend>,
    importer: Arc<BlockImporter>,
    client: Arc<GatewayProvider>,
}
impl PipelineSteps for ClassesSyncSteps {
    type InputItem = HashMap<Felt, DeclaredClassCompiledClass>;
    type SequentialStepInput = ();
    type Output = ();

    async fn parallel_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        input: Vec<Self::InputItem>,
    ) -> anyhow::Result<Self::SequentialStepInput> {
        if input.iter().all(|i| i.is_empty()) {
            return Ok(());
        }
        AbortOnDrop::spawn(async move {
            for (block_n, classes) in block_range.zip(input) {
                let mut declared_classes = vec![];
                for (&class_hash, &compiled_class_hash) in classes.iter() {
                    let class = self
                        .client
                        .get_class_by_hash(class_hash, BlockId::Number(block_n))
                        .await
                        .with_context(|| format!("Getting class_hash={class_hash:#x} with block_n={block_n}"))?;

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

                    declared_classes.push(ClassInfoWithHash { class_info, class_hash });
                }

                self.importer
                    .run_in_rayon_pool(move |importer| {
                        let classes = importer.verify_compile_classes(block_n, declared_classes, &classes)?;
                        importer.save_classes(block_n, classes)
                    })
                    .await?;
            }
            Ok(())
        })
        .await
    }

    async fn sequential_step(
        self: Arc<Self>,
        block_range: Range<u64>,
        _input: Self::SequentialStepInput,
    ) -> anyhow::Result<ApplyOutcome<Self::Output>> {
        tracing::debug!("gateway classes sequential step: {block_range:?}");
        if let Some(block_n) = block_range.last() {
            self.backend.head_status().classes.set(Some(block_n));
        }
        Ok(ApplyOutcome::Success(()))
    }

    fn starting_block_n(&self) -> Option<u64> {
        self.backend.head_status().latest_full_block_n()
    }
}
