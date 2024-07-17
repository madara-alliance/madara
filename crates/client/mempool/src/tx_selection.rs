use blockifier::execution::call_info::CallInfo;

use crate::Mempool;

// TODO: are there other limits to worry about here?
pub struct GasLimiter {
    max_l2_gas: u64,
    max_l1_gas: u64,
    current_l2_gas: u64,
    current_l1_gas: u64,
}

pub struct GasLimitExceeded;

impl GasLimiter {
    pub fn new(max_cairo_steps: u64, max_l1_gas: u64) -> Self {
        Self { max_l2_gas: max_cairo_steps, max_l1_gas, current_l2_gas: 0, current_l1_gas: 0 }
    }

    pub fn increment(&mut self, exec_info: &CallInfo) -> Result<(), GasLimitExceeded> {
        let new_l1_gas = self.current_l1_gas + exec_info.execution.gas_consumed;
        let new_cairo_steps = self.current_l2_gas + exec_info.resources.n_steps as u64;
        if new_l1_gas > self.max_l1_gas || new_cairo_steps > self.max_l2_gas {
            return Err(GasLimitExceeded);
        }

        self.current_l2_gas = new_cairo_steps;
        self.current_l1_gas = new_l1_gas;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.current_l2_gas = 0;
        self.current_l1_gas = 0;
    }
}

fn make_block(mempool: &mut Mempool) {
    let mut choosen_transactions = vec![];

    let mut gas_limiter = GasLimiter::new(mempool.config.cairo_steps_per_block, mempool.config.eth_gas_per_block);

    // Choose transactions to add to the chain
    while let Some(mut chain) = mempool.pop_next_nonce_chain() {
        let tx = chain.pop_head();

        // exec tx
        let exec_info = todo!();

        if gas_limiter.increment(&exec_info).is_err() {
            // Gas limit
            break;
        }

        choosen_transactions.push(tx);
        if !chain.is_empty() {
            mempool.re_add_nonce_chain(chain);
        }
    }
}
