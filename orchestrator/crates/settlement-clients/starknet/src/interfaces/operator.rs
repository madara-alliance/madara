use crate::utils::LocalWalletSignerMiddleware;
use crate::utils::{call_contract, invoke_contract};
use color_eyre::{eyre::eyre, Result};
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{Felt, InvokeTransactionResult};
use starknet::providers::jsonrpc::{HttpTransport, JsonRpcClient};

pub struct Operator {
    signer: LocalWalletSignerMiddleware,
    address: Felt,
}

impl Operator {
    pub fn new(address: Felt, signer: LocalWalletSignerMiddleware) -> Self {
        Self { signer, address }
    }

    fn provider(&self) -> &JsonRpcClient<HttpTransport> {
        self.signer.provider()
    }

    pub async fn register_operator(&self, new_operator: Felt) -> Result<InvokeTransactionResult> {
        invoke_contract(&self.signer, self.address, "register_operator", vec![new_operator]).await
    }

    pub async fn unregister_operator(&self, removed_operator: Felt) -> Result<InvokeTransactionResult> {
        invoke_contract(&self.signer, self.address, "unregister_operator", vec![removed_operator]).await
    }

    pub async fn is_operator(&self, operator: Felt) -> Result<bool> {
        let provider = self.provider();
        let values = call_contract(provider, self.address, "is_operator", vec![operator]).await?;

        values
            .first()
            .map(|value| *value != Felt::ZERO)
            .ok_or_else(|| eyre!("Contract error: expected at least one return value"))
    }

    pub async fn set_program_info(&self, program_hash: Felt, config_hash: Felt) -> Result<InvokeTransactionResult> {
        invoke_contract(&self.signer, self.address, "set_program_info", vec![program_hash, config_hash]).await
    }

    pub async fn get_program_info(&self) -> Result<(Felt, Felt)> {
        let provider = self.provider();
        let values = call_contract(provider, self.address, "get_program_info", vec![]).await?;

        values
            .first()
            .and_then(|first| values.get(1).map(|second| (*first, *second)))
            .ok_or_else(|| eyre!("Contract error: expected exactly two return values"))
    }

    pub async fn set_facts_registry(&self, facts_registry: Felt) -> Result<InvokeTransactionResult> {
        invoke_contract(&self.signer, self.address, "set_facts_registry", vec![facts_registry]).await
    }

    pub async fn get_facts_registry(&self) -> Result<Felt> {
        let provider = self.provider();
        let values = call_contract(provider, self.address, "get_facts_registry", vec![]).await?;

        values.first().cloned().ok_or_else(|| eyre!("Contract error: expected at least one return value"))
    }
}
