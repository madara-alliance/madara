use mp_convert::Felt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

const NO_CONTEXT_BLOCK: u64 = u64::MAX;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReadOp {
    GetStorageAt { block_n: u64, contract_address: Felt, key: Felt },
    GetContractNonceAt { block_n: u64, contract_address: Felt },
    GetContractClassHashAt { block_n: u64, contract_address: Felt },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ReadSource {
    Db,
    Override,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadEvent {
    pub op: ReadOp,
    pub value: Option<Felt>,
    pub source: ReadSource,
    pub context_block: Option<u64>,
}

pub trait ReadHook: Send + Sync + 'static {
    fn on_read(&self, event: ReadEvent);

    /// Optional override for reads. Return Some(value) to override the DB.
    /// Return None to fall back to the DB.
    fn override_read(&self, _op: &ReadOp) -> Option<Option<Felt>> {
        None
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadHookError {
    #[error("read hook already set")]
    AlreadySet,
}

static READ_HOOK: OnceLock<RwLock<Option<Arc<dyn ReadHook>>>> = OnceLock::new();
static CONTEXT_BLOCK: AtomicU64 = AtomicU64::new(NO_CONTEXT_BLOCK);

fn read_hook_cell() -> &'static RwLock<Option<Arc<dyn ReadHook>>> {
    READ_HOOK.get_or_init(|| RwLock::new(None))
}

pub fn set_read_hook(hook: Arc<dyn ReadHook>) -> Result<(), ReadHookError> {
    let mut guard = read_hook_cell().write().expect("read hook lock poisoned");
    if guard.is_some() {
        return Err(ReadHookError::AlreadySet);
    }
    *guard = Some(hook);
    Ok(())
}

pub fn clear_read_hook() {
    let mut guard = read_hook_cell().write().expect("read hook lock poisoned");
    *guard = None;
}

pub fn set_context_block(block_n: Option<u64>) {
    match block_n {
        Some(n) => CONTEXT_BLOCK.store(n, Ordering::Relaxed),
        None => CONTEXT_BLOCK.store(NO_CONTEXT_BLOCK, Ordering::Relaxed),
    }
}

pub fn current_context_block() -> Option<u64> {
    let n = CONTEXT_BLOCK.load(Ordering::Relaxed);
    if n == NO_CONTEXT_BLOCK {
        None
    } else {
        Some(n)
    }
}

pub(crate) fn maybe_override(op: &ReadOp) -> Option<Option<Felt>> {
    let guard = read_hook_cell().read().expect("read hook lock poisoned");
    guard.as_ref().and_then(|hook| hook.override_read(op))
}

pub(crate) fn record_read(op: ReadOp, value: Option<Felt>, source: ReadSource) {
    let guard = read_hook_cell().read().expect("read hook lock poisoned");
    if let Some(hook) = guard.as_ref() {
        let event = ReadEvent { op, value, source, context_block: current_context_block() };
        hook.on_read(event);
    }
}
