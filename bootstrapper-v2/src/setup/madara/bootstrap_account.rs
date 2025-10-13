use anyhow::Context;
use starknet::{
    accounts::{Account, AccountFactory, ExecutionEncoding, OpenZeppelinAccountFactory, SingleOwnerAccount},
    core::types::{
        contract::{CompiledClass, SierraClass},
        BlockId, BlockTag, Felt,
    },
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::{LocalWallet, SigningKey},
};
use std::sync::Arc;

use crate::utils::wait_for_transaction;

pub struct BootstrapAccount<'a> {
    // Bootstrap account used to make the first declaration
    account: SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    provider: &'a JsonRpcClient<HttpTransport>,
}

impl<'a> BootstrapAccount<'a> {
    pub fn new(provider: &'a JsonRpcClient<HttpTransport>, chain_id: Felt) -> Self {
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(
            Felt::from_hex("0x424f4f545354524150").context("Invalid bootstrap private key hex").unwrap(),
        ));

        let account = SingleOwnerAccount::new(
            provider.clone(),
            signer,
            BootstrapAccount::bootstrap_address(),
            chain_id,
            ExecutionEncoding::New,
        );

        Self { account, provider }
    }

    pub fn bootstrap_address() -> Felt {
        Felt::from_hex("0x424f4f545354524150").unwrap()
    }

    // A felt representation of the string 'BOOTSTRAP'.
    pub async fn bootstrap_declare(&self) -> anyhow::Result<()> {
        let contract_artifact: SierraClass = serde_json::from_reader(
            std::fs::File::open(
                "contracts/madara/target/dev/madara_factory_contracts_AccountUpgradeable.contract_class.json",
            )
            .context("Failed to open OpenZeppelin Account sierra file")?,
        )
        .context("Failed to read OpenZeppelin Account sierra file")?;

        let contract_casm_artifact: CompiledClass = serde_json::from_reader(
            std::fs::File::open(
                "contracts/madara/target/dev/madara_factory_contracts_AccountUpgradeable.compiled_contract_class.json",
            )
            .context("Failed to open OpenZeppelin Account casm file")?,
        )
        .context("Failed to read OpenZeppelin Account casm file")?;

        // Check if already declared
        if self
            .provider
            .get_class(BlockId::Tag(starknet::core::types::BlockTag::PreConfirmed), contract_artifact.class_hash()?)
            .await
            .is_ok()
        {
            log::info!("OpenZeppelin Account contract already declared, skipping declaration.");
            return Ok(());
        }

        // Class hash of the compiled CASM class
        let compiled_class_hash = contract_casm_artifact.class_hash()?;

        // We need to flatten the ABI into a string first
        let flattened_class = contract_artifact.clone().flatten().context("Failed to flatten contract artifact")?;

        let declaration = self
            .account
            .declare_v3(Arc::new(flattened_class), compiled_class_hash)
            .l1_gas(0)
            .l2_gas(0)
            .l1_data_gas(0)
            .nonce(Felt::ZERO);

        let result = declaration.send().await?;

        log::info!("Transaction hash: {:#064x}", result.transaction_hash);
        log::info!(
            "OpenZeppelin Account class hash: {:#064x} {:?} {:?}",
            result.class_hash,
            compiled_class_hash,
            contract_artifact.class_hash()?
        );

        wait_for_transaction(self.provider, result.transaction_hash, "OpenZeppelin Account Declaration").await?;
        log::info!("OpenZeppelin Account contract declared successfully !!");

        Ok(())
    }

    pub async fn deploy_account(
        &self,
        private_key: &str,
    ) -> anyhow::Result<SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>> {
        // Read the OpenZeppelin Account contract artifacts to get the class hash
        let contract_artifact: SierraClass = serde_json::from_reader(
            std::fs::File::open(
                "contracts/madara/target/dev/madara_factory_contracts_AccountUpgradeable.contract_class.json",
            )
            .context("Failed to open OpenZeppelin Account sierra file")?,
        )
        .context("Failed to read OpenZeppelin Account sierra file")?;

        // Get the class hash
        let class_hash = contract_artifact.class_hash()?;

        // Create a signer from the private key
        let signer = LocalWallet::from(SigningKey::from_secret_scalar(
            Felt::from_hex(private_key).context("Invalid private key format")?,
        ));

        let salt = Felt::from(0u64); // Salt for deployment

        // Create an OpenZeppelin account factory for deployment
        let account_factory =
            OpenZeppelinAccountFactory::new(class_hash, self.account.chain_id(), &signer, self.provider)
                .await
                .context("Failed to create OpenZeppelin account factory")?;

        // Deploy the account using the factory
        let deploy_result = account_factory
            .deploy_v3(salt)
            .l1_gas(0)
            .l2_gas(0)
            .l1_data_gas(0)
            .send()
            .await
            .context("Failed deploying OpenZeppelin account")?;

        wait_for_transaction(self.provider, deploy_result.transaction_hash, "OpenZeppelin Account Deployment").await?;
        log::info!("OpenZeppelin Account deployment successful!");
        log::info!("Transaction hash: {:#064x}", deploy_result.transaction_hash);
        log::info!("Account address: {:#064x}", deploy_result.contract_address);
        log::info!("Class hash: {:#064x}", class_hash);
        log::info!("Salt: {:#064x}", salt);

        // Create and return the new account instance
        let mut new_account = SingleOwnerAccount::new(
            self.provider.clone(),
            signer,
            deploy_result.contract_address,
            self.account.chain_id(),
            ExecutionEncoding::New,
        );

        new_account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

        Ok(new_account)
    }
}
