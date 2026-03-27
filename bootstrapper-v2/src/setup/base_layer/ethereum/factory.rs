use alloy::{
    network::TransactionBuilder, primitives::Address, providers::Provider, rpc::types::eth::TransactionRequest, sol,
    sol_types::SolConstructor,
};
use anyhow::{anyhow, Context};
use serde_json::Value;
use std::fs;
pub use Factory::BaseLayerContracts;
use Factory::{CoreContractInitData, FactoryInstance, ImplementationContracts};

use crate::setup::base_layer::{ethereum::error::EthereumError, BaseLayerError};

const FACTORY_ARTIFACT_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/ethereum/out/Factory.sol/Factory.json");
const PROXY_SETUP_ARTIFACT_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/ethereum/out/ProxySetup.sol/ProxySetup.json");

// Factory contract for deploying other contracts
sol!(
    #[allow(missing_docs)]
    #[sol(rpc, ignore_unlinked)]
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
    P: Provider,
{
    pub async fn deploy_new(
        provider: P,
        owner: Address,
        implementation_contracts: ImplementationContracts,
    ) -> anyhow::Result<Self> {
        let proxy_setup_address = deploy_artifact(&provider, PROXY_SETUP_ARTIFACT_PATH)
            .await
            .context("Failed to deploy ProxySetup library")?;
        let deploy_code = linked_factory_deploy_code(proxy_setup_address, owner, implementation_contracts)?;
        let deploy_tx = TransactionRequest::default().with_deploy_code(deploy_code);
        let pending_transaction = provider.send_transaction(deploy_tx).await?;
        let receipt = pending_transaction.get_receipt().await?;
        let factory_address = receipt.contract_address.ok_or_else(|| anyhow!("missing factory contract address"))?;
        let factory_contract = Factory::new(factory_address, provider);
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
        let preview = self.factory_contract.setup(core_contract_init_data.clone(), operator, governor).call().await?;
        let tx_hash =
            self.factory_contract.setup(core_contract_init_data, operator, governor).send().await?.watch().await?;
        log::info!("Factory setup transaction hash: {:?}", tx_hash);
        Ok(preview)
    }
}

async fn deploy_artifact<P: Provider>(provider: &P, artifact_path: &str) -> anyhow::Result<Address> {
    let artifact = read_artifact(artifact_path)?;
    let deploy_code = hex::decode(read_bytecode_hex(&artifact)?)?;
    let deploy_tx = TransactionRequest::default().with_deploy_code(deploy_code);
    let pending_transaction = provider.send_transaction(deploy_tx).await?;
    let receipt = pending_transaction.get_receipt().await?;
    receipt.contract_address.ok_or_else(|| anyhow!("missing contract address for artifact {artifact_path}"))
}

fn linked_factory_deploy_code(
    proxy_setup_address: Address,
    owner: Address,
    implementation_contracts: ImplementationContracts,
) -> anyhow::Result<Vec<u8>> {
    let artifact = read_artifact(FACTORY_ARTIFACT_PATH)?;
    let mut bytecode_hex = read_bytecode_hex(&artifact)?;
    let link_references = artifact["bytecode"]["linkReferences"]["src/factory/libraries/ProxySetup.sol"]["ProxySetup"]
        .as_array()
        .ok_or_else(|| anyhow!("missing ProxySetup link references in Factory artifact"))?;

    let linked_address = proxy_setup_address.to_string();
    let linked_address = linked_address.trim_start_matches("0x");

    for reference in link_references.iter().rev() {
        let start = reference["start"].as_u64().ok_or_else(|| anyhow!("missing ProxySetup link start"))? as usize * 2;
        let length =
            reference["length"].as_u64().ok_or_else(|| anyhow!("missing ProxySetup link length"))? as usize * 2;
        bytecode_hex.replace_range(start..start + length, linked_address);
    }

    let mut deploy_code = hex::decode(bytecode_hex)?;
    deploy_code.extend_from_slice(
        &Factory::constructorCall { owner, _implementationContracts: implementation_contracts }.abi_encode(),
    );
    Ok(deploy_code)
}

fn read_artifact(path: &str) -> anyhow::Result<Value> {
    let artifact = fs::read_to_string(path).with_context(|| format!("failed to read artifact {path}"))?;
    serde_json::from_str(&artifact).with_context(|| format!("failed to parse artifact JSON {path}"))
}

fn read_bytecode_hex(artifact: &Value) -> anyhow::Result<String> {
    artifact["bytecode"]["object"]
        .as_str()
        .ok_or_else(|| anyhow!("artifact bytecode.object missing"))?
        .strip_prefix("0x")
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("artifact bytecode.object missing 0x prefix"))
}
