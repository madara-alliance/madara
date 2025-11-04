use blockifier::execution::contract_class::RunnableCompiledClass;
use cairo_vm::types::errors::program_errors::ProgramError;
use serde::de::Error as _;
use starknet_api::contract_class::ContractClass as ApiContractClass;
use std::sync::Arc;

use crate::{ConvertedClass, LegacyConvertedClass, SierraConvertedClass};

#[cfg(feature = "cairo_native")]
use {
    blockifier::execution::native::contract_class::NativeCompiledClassV1,
    cairo_native::executor::AotContractExecutor,
    dashmap::DashMap,
    std::path::{Path, PathBuf},
    tokio::sync::RwLock,
};

#[cfg(feature = "cairo_native")]
static NATIVE_CACHE: std::sync::LazyLock<
    DashMap<starknet_types_core::felt::Felt, Arc<NativeCompiledClassV1>>,
> = std::sync::LazyLock::new(DashMap::new);

#[cfg(feature = "cairo_native")]
static COMPILATION_IN_PROGRESS: std::sync::LazyLock<
    DashMap<starknet_types_core::felt::Felt, Arc<RwLock<()>>>,
> = std::sync::LazyLock::new(DashMap::new);

#[cfg(feature = "cairo_native")]
fn get_native_cache_path(class_hash: &starknet_types_core::felt::Felt) -> PathBuf {
    // Try environment variable first, fall back to default
    let base_path = std::env::var("MADARA_NATIVE_CLASSES_PATH")
        .unwrap_or_else(|_| "/usr/share/madara/data/classes".to_string());
    
    static LOGGED_PATH: std::sync::Once = std::sync::Once::new();
    LOGGED_PATH.call_once(|| {
        tracing::info!("🚀 Cairo Native feature enabled - native classes cache directory: {}", base_path);
    });
    
    Path::new(&base_path).join(format!("{:#x}.so", class_hash))
}

#[cfg(feature = "cairo_native")]
fn spawn_native_compilation(
    class_hash: starknet_types_core::felt::Felt,
    sierra: Arc<SierraConvertedClass>,
) {
    // Spawn background task for native compilation
    tokio::spawn(async move {
        let lock = COMPILATION_IN_PROGRESS
            .entry(class_hash)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone();
        let _guard = lock.write().await;

        // Check cache again in case another task compiled it
        if NATIVE_CACHE.contains_key(&class_hash) {
            COMPILATION_IN_PROGRESS.remove(&class_hash);
            return;
        }

        let path = get_native_cache_path(&class_hash);
        
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let start = std::time::Instant::now();
        tracing::info!(
            "🔧 [Cairo Native] Starting background compilation for class {:#x} -> {}",
            class_hash,
            path.display()
        );

        let sierra_clone = sierra.clone();
        let path_clone = path.clone();
        match tokio::task::spawn_blocking(move || {
            sierra_clone.info.contract_class.compile_to_native(&path_clone)
        })
        .await
        {
            Ok(Ok(executor)) => {
                let elapsed = start.elapsed();
                tracing::info!(
                    "✅ [Cairo Native] Compilation successful for class {:#x} in {:?} - saved to disk",
                    class_hash,
                    elapsed
                );

                // Get sierra version and compiled class
                if let (Ok(sierra_version), Ok(casm)) = (
                    sierra.info.contract_class.sierra_version(),
                    sierra.compiled.as_ref().try_into(),
                ) {
                    // Convert (CasmContractClass, SierraVersion) tuple to CompiledClassV1
                    if let Ok(blockifier_compiled_class) = (casm, sierra_version).try_into() {
                        let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);
                        NATIVE_CACHE.insert(class_hash, Arc::new(native_class));
                        let cache_size = NATIVE_CACHE.len();
                        tracing::info!(
                            "💾 [Cairo Native] Cached native class {:#x} in memory (cache size: {})",
                            class_hash,
                            cache_size
                        );
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "⚠️  [Cairo Native] Compilation failed for class {:#x}: {:#}",
                    class_hash,
                    e
                );
            }
            Err(e) => {
                tracing::error!(
                    "❌ [Cairo Native] Compilation task panicked for class {:#x}: {:#}",
                    class_hash,
                    e
                );
            }
        }

        COMPILATION_IN_PROGRESS.remove(&class_hash);
    });
}

impl TryFrom<&ConvertedClass> for RunnableCompiledClass {
    type Error = ProgramError;

    fn try_from(converted_class: &ConvertedClass) -> Result<Self, Self::Error> {
        match converted_class {
            ConvertedClass::Legacy(LegacyConvertedClass { info, .. }) => {
                RunnableCompiledClass::try_from(ApiContractClass::V0(info.contract_class.to_starknet_api_no_abi()?))
            }
            ConvertedClass::Sierra(sierra @ SierraConvertedClass { class_hash, compiled, info }) => {
                #[cfg(not(feature = "cairo_native"))]
                {
                    let sierra_version = info.contract_class.sierra_version().map_err(|_| {
                        ProgramError::Parse(serde_json::Error::custom("Failed to get sierra version from program"))
                    })?;
                    RunnableCompiledClass::try_from(ApiContractClass::V1((compiled.as_ref().try_into()?, sierra_version)))
                }

                #[cfg(feature = "cairo_native")]
                {
                    // Check in-memory cache first
                    if let Some(cached) = NATIVE_CACHE.get(class_hash) {
                        let cache_size = NATIVE_CACHE.len();
                        tracing::info!(
                            "⚡ [Cairo Native] Using in-memory cached native class {:#x} (cache size: {})",
                            class_hash,
                            cache_size
                        );
                        return Ok(RunnableCompiledClass::from(cached.value().as_ref().clone()));
                    }

                    // Try to load from disk synchronously (fast path)
                    let path = get_native_cache_path(class_hash);
                    if let Ok(Some(executor)) = AotContractExecutor::from_path(&path) {
                        tracing::info!(
                            "📁 [Cairo Native] Loaded native class {:#x} from disk: {}",
                            class_hash,
                            path.display()
                        );
                        
                        let sierra_version = info.contract_class.sierra_version().map_err(|_| {
                            ProgramError::Parse(serde_json::Error::custom("Failed to get sierra version from program"))
                        })?;
                        let casm: casm_classes_v2::casm_contract_class::CasmContractClass = compiled.as_ref().try_into().map_err(|_| {
                            ProgramError::Parse(serde_json::Error::custom("Failed to convert to CASM"))
                        })?;
                        
                        // Convert (CasmContractClass, SierraVersion) tuple to CompiledClassV1
                        let blockifier_compiled_class = (casm, sierra_version).try_into()
                            .map_err(|_| ProgramError::Parse(serde_json::Error::custom("Failed to convert to blockifier class")))?;
                        let native_class = NativeCompiledClassV1::new(executor, blockifier_compiled_class);
                        
                        NATIVE_CACHE.insert(*class_hash, Arc::new(native_class.clone()));
                        tracing::debug!("[Cairo Native] Added class {:#x} to in-memory cache", class_hash);
                        return Ok(RunnableCompiledClass::from(native_class));
                    }

                    // Not in cache and not on disk - spawn async compilation and fall back to VM
                    let is_already_compiling = COMPILATION_IN_PROGRESS.contains_key(class_hash);
                    if is_already_compiling {
                        tracing::info!(
                            "🔄 [Cairo Native] Class {:#x} already compiling, using VM for now",
                            class_hash
                        );
                    } else {
                        tracing::info!(
                            "🔄 [Cairo Native] Class {:#x} not cached - spawning background compilation, using VM for now",
                            class_hash
                        );
                        spawn_native_compilation(*class_hash, Arc::new(sierra.clone()));
                    }

                    // Fall back to cairo-vm (CASM execution)
                    tracing::debug!("🐪 [Cairo VM] Executing class {:#x} with Cairo VM", class_hash);
                    let sierra_version = info.contract_class.sierra_version().map_err(|_| {
                        ProgramError::Parse(serde_json::Error::custom("Failed to get sierra version from program"))
                    })?;
                    RunnableCompiledClass::try_from(ApiContractClass::V1((compiled.as_ref().try_into()?, sierra_version)))
                }
            }
        }
    }
}
