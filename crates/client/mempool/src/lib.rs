use std::cmp;
use std::collections::BTreeSet;
use std::time::Duration;

use blockifier::execution::call_info::CallInfo;
use serde::{Deserialize, Serialize};
use starknet_core::types::{BroadcastedInvokeTransactionV3, Felt};

mod tx_selection;

/// Sequencer specific constants
/// See https://docs.starknet.io/tools/limits-and-triggers
#[derive(Serialize, Deserialize, Clone)]
pub struct MempoolChainConfig {
    pub block_time: Duration,
    pub cairo_steps_per_block: u64,
    pub eth_gas_per_block: u64,
    pub cairo_steps_per_tx: u64,
    pub cairo_steps_validate: u64,
    pub contract_bytecode_sz_felts: u64,
    pub contract_class_sz: u64,
    pub ip_address_rw_limit: u64,
    pub signature_sz_felts: u64,
    pub calldata_sz_felts: u64,
}

// TODO: deserialize that from a config file.
pub const MEMPOOL_CHAIN_CONFIG: MempoolChainConfig = MempoolChainConfig {
    block_time: Duration::from_secs(60 * 6),
    cairo_steps_per_block: 40_000_000,
    eth_gas_per_block: 5_000_000,
    cairo_steps_per_tx: 4_000_000,
    cairo_steps_validate: 1_000_000,
    contract_bytecode_sz_felts: 81_290,
    contract_class_sz: 4_089_446,
    ip_address_rw_limit: 200,
    signature_sz_felts: 4_000,
    calldata_sz_felts: 4_000,
};

pub struct MempoolTransaction {
    tip: u128,
    sender_address: Felt,
    nonce: u64,
    tx_hash: Felt,
}

struct OrderMempoolTransactionByNonce(MempoolTransaction);

impl PartialEq for OrderMempoolTransactionByNonce {
    fn eq(&self, other: &Self) -> bool {
        self.0.tx_hash == other.0.tx_hash
    }
}
impl Eq for OrderMempoolTransactionByNonce {}

impl Ord for OrderMempoolTransactionByNonce {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.nonce.cmp(&other.0.nonce)
    }
}
impl PartialOrd for OrderMempoolTransactionByNonce {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

pub struct NonceChain {
    contract_addr: Felt,
    transactions: BTreeSet<OrderMempoolTransactionByNonce>,
}

impl NonceChain {
    /// # Panics
    ///
    /// Panics if the NonceChain is empty.
    pub fn head_tip(&self) -> u128 {
        self.transactions.first().expect("no transaction for this queue item").0.tip
    }

    /// # Panics
    ///
    /// Panics if the NonceChain is empty.
    pub fn pop_head(&mut self) -> MempoolTransaction {
        self.transactions.pop_first().map(|tx| tx.0).expect("no transaction for this queue item")
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }
}

struct NonceChainByNextTip(NonceChain);
impl PartialEq for NonceChainByNextTip {
    fn eq(&self, other: &Self) -> bool {
        self.0.contract_addr == other.0.contract_addr
    }
}
impl Eq for NonceChainByNextTip {}

impl Ord for NonceChainByNextTip {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.0.head_tip().cmp(&other.0.head_tip())
    }
}
impl PartialOrd for NonceChainByNextTip {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub struct Mempool {
    tx_queue: BTreeSet<NonceChainByNextTip>,
    config: MempoolChainConfig,
}

impl Mempool {
    pub fn new(config: MempoolChainConfig) -> Self {
        Mempool { tx_queue: BTreeSet::new(), config }
    }

    pub fn accept_tx(tx: BroadcastedInvokeTransactionV3) {
        // 1. __validate__ and basic validation of the transaction
        // * Early reject when fees are too low.

        // TODO

        // TODO: Stop here if only_query = true.

        // 2. Add it to the nonce chain for the account nonce

    }

    pub fn pop_next_nonce_chain(&mut self) -> Option<NonceChain> {
        self.tx_queue.pop_first().map(|el| el.0)
    }
    pub fn re_add_nonce_chain(&mut self, chain: NonceChain) {
        self.tx_queue.insert(NonceChainByNextTip(chain));
    }
}