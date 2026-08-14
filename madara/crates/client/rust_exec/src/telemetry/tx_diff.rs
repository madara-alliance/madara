use std::cell::Cell;

use starknet_types_core::felt::Felt;

#[derive(Clone, Copy)]
pub struct TxDiffContext {
    pub block_number: u64,
    pub tx_hash: Felt,
}

thread_local! {
    static CONTEXT: Cell<Option<TxDiffContext>> = const { Cell::new(None) };
}

pub struct TxDiffContextGuard(Option<TxDiffContext>);

impl Drop for TxDiffContextGuard {
    fn drop(&mut self) {
        CONTEXT.set(self.0);
    }
}

pub fn enter(block_number: u64, tx_hash: Felt) -> TxDiffContextGuard {
    TxDiffContextGuard(CONTEXT.replace(Some(TxDiffContext { block_number, tx_hash })))
}

pub fn current() -> Option<TxDiffContext> {
    CONTEXT.get()
}
