use crate::utils::invoke_contract;
use crate::utils::LocalWalletSignerMiddleware;
use color_eyre::Result;
use starknet::core::types::{Felt, InvokeTransactionResult};

pub struct CoreContract {
    signer: LocalWalletSignerMiddleware,
    address: Felt,
}

impl CoreContract {
    pub fn new(address: Felt, signer: LocalWalletSignerMiddleware) -> Self {
        Self { signer, address }
    }

    pub async fn update_state(&self, snos_output: Vec<Felt>) -> Result<InvokeTransactionResult> {
        let mut calldata = Vec::with_capacity(snos_output.len() + 1);
        calldata.push(Felt::from(snos_output.len()));
        calldata.extend(snos_output);

        invoke_contract(&self.signer, self.address, "update_state", calldata).await
    }
}
