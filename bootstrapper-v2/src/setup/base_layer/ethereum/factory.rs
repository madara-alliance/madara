use alloy::{primitives::Address, sol};
use anyhow::Result;
use futures_util::StreamExt;
pub use Factory::BaseLayerContracts;
use Factory::{CoreContractInitData, FactoryInstance, ImplementationContracts};

// Factory contract for deploying other contracts
sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    Factory,
    "contracts/ethereum/out/Factory.sol/Factory.json"
);

#[allow(unused)]
pub struct FactoryDeploy<P> {
    pub factory_contract: FactoryInstance<P>,
}

#[allow(unused)]
impl<P> FactoryDeploy<P>
where
    P: alloy::providers::Provider,
{
    pub async fn new(provider: P, owner: Address, implementation_contracts: ImplementationContracts) -> Result<Self> {
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
    ) -> Result<BaseLayerContracts> {
        let contract_deployed_filter = self.factory_contract.BaseLayerContractsDeployed_filter().watch().await?;
        let tx_hash =
            self.factory_contract.setup(core_contract_init_data, operator, governor).send().await?.watch().await?;
        log::info!("Factory setup transaction hash: {:?}", tx_hash);

        // Wait for the BaseLayerContractsDeployed event with 5-minute timeout
        let mut event_stream = contract_deployed_filter.into_stream().take(1);
        let event = tokio::time::timeout(
            std::time::Duration::from_secs(300), // 5 minutes
            event_stream.next()
        )
        .await
        .map_err(|_| anyhow::anyhow!("Timeout waiting for BaseLayerContractsDeployed event after 5 minutes"))?
        .ok_or_else(|| anyhow::anyhow!("Failed to receive BaseLayerContractsDeployed event"))?;

        match event {
            Ok((event_data, log)) => {
                log::info!("Received BaseLayerContractsDeployed event: {:?}", log);
                log::info!("Base layer contracts: {:?}", event_data);
                // Extract the BaseLayerContracts data from the event
                Ok(event_data._baseLayerContracts)
            }
            Err(e) => Err(anyhow::anyhow!("Error receiving event: {:?}", e)),
        }
    }
}
