use std::time::Duration;

use serde::Serialize;
use starknet::core::types::Felt;
use tokio::time::sleep;

use crate::contract_clients::utils::{declare_contract, DeclarationInput, RpcAccount};
use crate::utils::constants::{ARGENT_ACCOUNT_CASM_PATH, ARGENT_ACCOUNT_SIERRA_PATH};
use crate::utils::{save_to_json, JsonValueType};

pub struct ArgentSetup<'a> {
    account: RpcAccount<'a>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArgentSetupOutput {
    pub argent_class_hash: Felt,
}

impl<'a> ArgentSetup<'a> {
    pub fn new(account: RpcAccount<'a>) -> Self {
        Self { account }
    }

    pub async fn setup(&self) -> ArgentSetupOutput {
        let argent_class_hash = declare_contract(DeclarationInput::DeclarationInputs(
            String::from(ARGENT_ACCOUNT_SIERRA_PATH),
            String::from(ARGENT_ACCOUNT_CASM_PATH),
            self.account.clone(),
        ))
        .await;
        log::info!("📣 Argent Hash Declared");
        save_to_json("argent_class_hash", &JsonValueType::StringType(argent_class_hash.to_string())).unwrap();
        sleep(Duration::from_secs(10)).await;

        ArgentSetupOutput { argent_class_hash }
    }
}
