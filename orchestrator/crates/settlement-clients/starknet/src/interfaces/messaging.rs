use crate::utils::invoke_contract;
use crate::utils::LocalWalletSignerMiddleware;
use color_eyre::Result;
use starknet::core::types::{Felt, InvokeTransactionResult};

pub struct Messaging {
    signer: LocalWalletSignerMiddleware,
    address: Felt,
}

impl Messaging {
    pub fn new(address: Felt, signer: LocalWalletSignerMiddleware) -> Self {
        Self { signer, address }
    }

    pub async fn send_message_to_appchain(
        &self,
        to_address: Felt,
        selector: Felt,
        payload: Vec<Felt>,
    ) -> Result<InvokeTransactionResult> {
        let mut calldata = Vec::with_capacity(payload.len() + 2);
        calldata.push(to_address);
        calldata.push(selector);
        calldata.extend(payload);

        invoke_contract(
            &self.signer,
            self.address,
            "send_message_to_appchain",
            calldata,
        )
        .await
    }

    pub async fn consume_message_from_appchain(
        &self,
        from_address: Felt,
        payload: Vec<Felt>,
    ) -> Result<InvokeTransactionResult> {
        let mut calldata = Vec::with_capacity(payload.len() + 1);
        calldata.push(from_address);
        calldata.extend(payload);

        invoke_contract(
            &self.signer,
            self.address,
            "consume_message_from_appchain",
            calldata,
        )
        .await
    }

    pub async fn start_message_cancellation(
        &self,
        to_address: Felt,
        selector: Felt,
        nonce: Felt,
        payload: Vec<Felt>,
    ) -> Result<InvokeTransactionResult> {
        let mut calldata = Vec::with_capacity(payload.len() + 3);
        calldata.push(to_address);
        calldata.push(selector);
        calldata.push(nonce);
        calldata.extend(payload);

        invoke_contract(
            &self.signer,
            self.address,
            "start_message_cancellation",
            calldata,
        )
        .await
    }

    pub async fn cancel_message(
        &self,
        to_address: Felt,
        selector: Felt,
        nonce: Felt,
        payload: Vec<Felt>,
    ) -> Result<InvokeTransactionResult> {
        let mut calldata = Vec::with_capacity(payload.len() + 3);
        calldata.push(to_address);
        calldata.push(selector);
        calldata.push(nonce);
        calldata.extend(payload);

        invoke_contract(&self.signer, self.address, "cancel_message", calldata).await
    }
}
