use alloy::{primitives::Address, sol};
use anyhow::Result;
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
    ) -> Result<()> {
        let tx_hash =
            self.factory_contract.setup(core_contract_init_data, operator, governor).send().await?.watch().await?;
        println!("Factory setup transaction hash: {:?}", tx_hash);
        Ok(())
    }
}
