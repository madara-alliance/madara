use alloy::{primitives::Address, sol};
use futures_util::StreamExt;
pub use Factory::BaseLayerContracts;
use Factory::{CoreContractInitData, FactoryInstance, ImplementationContracts};

use crate::setup::base_layer::{ethereum::error::EthereumError, BaseLayerError};

// Factory contract for deploying other contracts
sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    Factory,
    "contracts/ethereum/out/Factory.sol/Factory.json"
);

pub struct DeployedFactory<P> {
    pub factory_contract: FactoryInstance<P>,
}

#[derive(thiserror::Error, Debug)]
pub enum FactorySetupError {
    #[error("Alloy rpc error: {0}")]
    AlloyRpcError(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),

    #[error("Alloy Contract error: {0}")]
    AlloyContractError(#[from] alloy::contract::Error),

    #[error("Alloy provider error: {0}")]
    AlloyPendingTransactionError(#[from] alloy::providers::PendingTransactionError),

    #[error("Tokio timeout waiting for BaseLayerContractsDeployed event")]
    TimeOutExpired(#[from] tokio::time::error::Elapsed),

    #[error("Failed to receive BaseLayerContractsDeployed event")]
    EventNotEmitted,

    #[error("Failed to parse event")]
    FailedToParseEvent(#[from] alloy::sol_types::Error),
}

impl From<FactorySetupError> for BaseLayerError {
    fn from(value: FactorySetupError) -> Self {
        Self::Internal(Box::new(EthereumError::FactorySetupFailed(value)))
    }
}

impl<P> DeployedFactory<P>
where
    P: alloy::providers::Provider,
{
    pub async fn deploy_new(
        provider: P,
        owner: Address,
        implementation_contracts: ImplementationContracts,
    ) -> anyhow::Result<Self> {
        let factory_contract = Factory::deploy(provider, owner, implementation_contracts).await?;
        Ok(Self { factory_contract })
    }

    pub fn address(&self) -> Address {
        *self.factory_contract.address()
    }

    pub async fn setup(
        &self,
        core_contract_init_data: CoreContractInitData,
        operator: Address,
        governor: Address,
    ) -> Result<BaseLayerContracts, FactorySetupError> {
        let contract_deployed_filter = self.factory_contract.BaseLayerContractsDeployed_filter().watch().await?;
        let tx_hash =
            self.factory_contract.setup(core_contract_init_data, operator, governor).send().await?.watch().await?;
        log::info!("Factory setup transaction hash: {:?}", tx_hash);

        // Wait for the BaseLayerContractsDeployed event with 5-minute timeout
        let mut event_stream = contract_deployed_filter.into_stream().take(1);
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(300), // 5 minutes
            event_stream.next(),
        )
        .await?
        .ok_or(FactorySetupError::EventNotEmitted)?;

        let (event_data, _log) = event?;
        log::info!(
            "Received BaseLayerContractsDeployed event, Base layer contracts: {:?}",
            event_data._baseLayerContracts
        );

        // Extract the BaseLayerContracts data from the event
        Ok(event_data._baseLayerContracts)
    }
}
