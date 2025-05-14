use std::time::Duration;

use serde::Serialize;
use starknet::accounts::ConnectedAccount;
use starknet::core::types::Felt;
use tokio::time::sleep;

use crate::contract_clients::config::Clients;
use crate::contract_clients::utils::{declare_contract, DeclarationInput, RpcAccount};
use crate::helpers::account_actions::{get_contract_address_from_deploy_tx, AccountActions};
use crate::utils::constants::UDC_PATH;
use crate::utils::{save_to_json, wait_for_transaction, JsonValueType};
use crate::ConfigFile;

pub struct UdcSetup<'a> {
    account: RpcAccount<'a>,
    account_address: Felt,
    arg_config: &'a ConfigFile,
    clients: &'a Clients,
}

#[derive(Debug, Clone, Serialize)]
pub struct UdcSetupOutput {
    pub udc_class_hash: Felt,
    pub udc_address: Felt,
}

impl<'a> UdcSetup<'a> {
    pub fn new(
        account: RpcAccount<'a>,
        account_address: Felt,
        arg_config: &'a ConfigFile,
        clients: &'a Clients,
    ) -> Self {
        Self { account, account_address, arg_config, clients }
    }

    pub async fn setup(&self) -> UdcSetupOutput {
        let udc_class_hash = declare_contract(DeclarationInput::LegacyDeclarationInputs(
            String::from(UDC_PATH),
            self.arg_config.rollup_declare_v0_seq_url.clone(),
            self.clients.provider_l2(),
        ))
        .await;
        log::info!("📣 UDC Class Hash Declared.");
        save_to_json("udc_class_hash", &JsonValueType::StringType(udc_class_hash.to_string())).unwrap();
        sleep(Duration::from_secs(10)).await;

        let txn = self
            .account
            .invoke_contract(
                self.account_address,
                "deploy_contract",
                Vec::from([udc_class_hash, Felt::ZERO, Felt::ONE, Felt::ZERO]),
                None,
            )
            .send()
            .await
            .unwrap();
        wait_for_transaction(
            self.account.provider(),
            txn.transaction_hash,
            "deploy_non_bridge_contracts : deploy_contract : udc",
        )
        .await
        .unwrap();
        let udc_address = get_contract_address_from_deploy_tx(self.account.provider(), &txn).await.unwrap();
        save_to_json("udc_address", &JsonValueType::StringType(udc_address.to_string())).unwrap();
        log::info!("📣 udc_address : {:?}", udc_address);

        UdcSetupOutput { udc_class_hash, udc_address }
    }
}
