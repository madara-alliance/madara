use crate::contract_clients::config::Clients;
use crate::contract_clients::utils::{
    build_single_owner_account, declare_contract, deploy_account_using_priv_key, DeclarationInput, RpcAccount,
    TEMP_ACCOUNT_PRIV_KEY,
};
use crate::utils::constants::{OZ_ACCOUNT_CASM_PATH, OZ_ACCOUNT_PATH, OZ_ACCOUNT_SIERRA_PATH};
use crate::utils::{convert_to_hex, save_to_json, JsonValueType};
use crate::ConfigFile;

pub async fn account_init<'a>(clients: &'a Clients, arg_config: &'a ConfigFile) -> RpcAccount<'a> {
    // >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>
    // Making temp account for declaration of OZ account Cairo 1 contract
    let oz_account_class_hash =
        declare_contract(clients, DeclarationInput::LegacyDeclarationInputs(String::from(OZ_ACCOUNT_PATH))).await;
    log::info!("OZ Account Class Hash Declared");
    save_to_json("oz_account_class_hash", &JsonValueType::StringType(oz_account_class_hash.to_string())).unwrap();

    log::info!("Waiting for block to be produced [/]");

    let account_address_temp =
        deploy_account_using_priv_key(TEMP_ACCOUNT_PRIV_KEY.to_string(), clients.provider_l2(), oz_account_class_hash)
            .await;

    let user_account_temp = build_single_owner_account(
        clients.provider_l2(),
        TEMP_ACCOUNT_PRIV_KEY,
        &convert_to_hex(&account_address_temp.to_string()),
        false,
    )
    .await;
    let oz_account_caio_1_class_hash = declare_contract(
        clients,
        DeclarationInput::DeclarationInputs(
            OZ_ACCOUNT_SIERRA_PATH.to_string(),
            OZ_ACCOUNT_CASM_PATH.to_string(),
            user_account_temp.clone(),
        ),
    )
    .await;
    save_to_json("oz_account_caio_1_class_hash", &JsonValueType::StringType(oz_account_caio_1_class_hash.to_string()))
        .unwrap();
    // >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>

    // Using Account Cairo 1 contract
    // >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>
    let account_address = deploy_account_using_priv_key(
        arg_config.rollup_priv_key.clone(),
        clients.provider_l2(),
        oz_account_caio_1_class_hash,
    )
    .await;
    save_to_json("account_address", &JsonValueType::StringType(account_address.to_string())).unwrap();
    build_single_owner_account(
        clients.provider_l2(),
        &arg_config.rollup_priv_key,
        &convert_to_hex(&account_address.to_string()),
        false,
    )
    .await
    // >>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>
}
