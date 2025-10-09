use super::{builder::GatewayProvider, request_builder::RequestBuilder};
use mp_block::{BlockId, BlockTag};
use mp_class::{ContractClass, FlattenedSierraClass};
use mp_gateway::block::ProviderBlockPreConfirmed;
use mp_gateway::error::{SequencerError, StarknetError};
use mp_gateway::user_transaction::{
    AddDeclareTransactionResult, AddDeployAccountTransactionResult, AddInvokeTransactionResult,
};
use mp_gateway::{
    block::{ProviderBlock, ProviderBlockHeader, ProviderBlockSignature},
    state_update::{ProviderStateUpdate, ProviderStateUpdateWithBlock},
    user_transaction::{
        UserDeclareTransaction, UserDeployAccountTransaction, UserInvokeFunctionTransaction, UserTransaction,
    },
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use starknet_core::types::contract::legacy::LegacyContractClass;
use starknet_types_core::felt::Felt;
use std::{borrow::Cow, sync::Arc};

const MAX_RETRIES: usize = 10;

impl GatewayProvider {
    /// Generic retry mechanism for GET requests
    async fn retry_get<T, F, Fut>(&self, request_fn: F) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, SequencerError>>,
    {
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            match request_fn().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < MAX_RETRIES - 1 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100 * (attempt as u64 + 1))).await;
                    }
                }
            }
        }
        Err(last_error.unwrap())
    }

    pub async fn get_block(&self, block_id: BlockId) -> Result<ProviderBlock, SequencerError> {
        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_block")
                .expect("Failed to add URI segment. This should not fail in prod.")
                .with_block_id(&block_id);

            request.send_get::<ProviderBlock>().await
        }).await
    }

    pub async fn get_preconfirmed_block(&self, block_number: u64) -> Result<ProviderBlockPreConfirmed, SequencerError> {
        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_preconfirmed_block")
                .expect("Failed to add URI segment. This should not fail in prod.")
                .with_block_id(&BlockId::Number(block_number));

            request.send_get::<ProviderBlockPreConfirmed>().await
        }).await
    }

    pub async fn get_header(&self, block_id: BlockId) -> Result<ProviderBlockHeader, SequencerError> {
        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_block")
                .expect("Failed to add URI segment. This should not fail in prod.")
                .with_block_id(&block_id)
                .add_param("headerOnly", "true");

            request.send_get::<ProviderBlockHeader>().await
        }).await
    }

    pub async fn get_state_update(&self, block_id: BlockId) -> Result<ProviderStateUpdate, SequencerError> {
        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_state_update")
                .expect("Failed to add URI segment. This should not fail in prod")
                .with_block_id(&block_id);

            request.send_get::<ProviderStateUpdate>().await
        }).await
    }

    pub async fn get_state_update_with_block(
        &self,
        block_id: BlockId,
    ) -> Result<ProviderStateUpdateWithBlock, SequencerError> {
        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_state_update")
                .expect("Failed to add URI segment. This should not fail in prod")
                .with_block_id(&block_id)
                .add_param(Cow::from("includeBlock"), "true");

            request.send_get::<ProviderStateUpdateWithBlock>().await
        }).await
    }

    pub async fn get_signature(&self, block_id: BlockId) -> Result<ProviderBlockSignature, SequencerError> {
        if matches!(block_id, BlockId::Tag(BlockTag::Pending)) {
            return Err(StarknetError::no_signature_for_pending_block().into());
        }

        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_signature")
                .expect("Failed to add URI segment. This should not fail in prod")
                .with_block_id(&block_id);

            request.send_get::<ProviderBlockSignature>().await
        }).await
    }

    pub async fn get_class_by_hash(
        &self,
        class_hash: Felt,
        block_id: BlockId,
    ) -> Result<ContractClass, SequencerError> {
        self.retry_get(|| async {
            let request = RequestBuilder::new(&self.client, self.feeder_gateway_url.clone(), self.headers.clone())
                .add_uri_segment("get_class_by_hash")
                .expect("Failed to add URI segment. This should not fail in prod.")
                .with_block_id(&block_id)
                .with_class_hash(class_hash);

            let value = request.send_get::<Value>().await?;

            if value.get("sierra_program").is_some() {
                let sierra: FlattenedSierraClass = serde_json::from_value(value)?;
                Ok(ContractClass::Sierra(Arc::new(sierra)))
            } else if value.get("program").is_some() {
                let legacy: LegacyContractClass = serde_json::from_value(value)?;
                Ok(ContractClass::Legacy(Arc::new(legacy.compress()?.into())))
            } else {
                let err = serde::de::Error::custom("Unknown contract type".to_string());
                Err(SequencerError::DeserializeBody { serde_error: err })
            }
        }).await
    }

    async fn add_transaction<T>(&self, transaction: UserTransaction) -> Result<T, SequencerError>
    where
        T: DeserializeOwned,
    {
        let request = RequestBuilder::new(&self.client, self.gateway_url.clone(), self.headers.clone())
            .add_uri_segment("add_transaction")
            .expect("Failed to add URI segment. This should not fail in prod.");

        request.send_post(transaction).await
    }

    pub async fn add_validated_transaction(
        &self,
        transaction: mp_transactions::validated::ValidatedTransaction,
    ) -> Result<(), SequencerError> {
        let url = self.madara_specific_url.as_ref().ok_or(SequencerError::NoUrl)?;

        let request = RequestBuilder::new(&self.client, url.clone(), self.headers.clone())
            .add_uri_segment("trusted_add_validated_transaction")
            .expect("Failed to add URI segment. This should not fail in prod.");

        request.send_post_bincode(transaction).await
    }

    pub async fn add_invoke_transaction(
        &self,
        transaction: UserInvokeFunctionTransaction,
    ) -> Result<AddInvokeTransactionResult, SequencerError> {
        self.add_transaction(UserTransaction::InvokeFunction(transaction)).await
    }

    pub async fn add_declare_transaction(
        &self,
        transaction: UserDeclareTransaction,
    ) -> Result<AddDeclareTransactionResult, SequencerError> {
        self.add_transaction(UserTransaction::Declare(transaction)).await
    }

    pub async fn add_deploy_account_transaction(
        &self,
        transaction: UserDeployAccountTransaction,
    ) -> Result<AddDeployAccountTransactionResult, SequencerError> {
        self.add_transaction(UserTransaction::DeployAccount(transaction)).await
    }
}
