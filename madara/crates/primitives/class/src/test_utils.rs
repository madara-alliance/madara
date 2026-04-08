use crate::{ContractClass, FlattenedSierraClass, LegacyContractClass};
use reqwest::Client;
use starknet_core::types::FlattenedSierraClass as CoreFlattenedSierraClass;
use std::sync::Arc;

pub(crate) const MAINNET_FEEDER_GATEWAY_URL: &str = "https://feeder.alpha-mainnet.starknet.io/feeder_gateway";
pub(crate) const SEPOLIA_FEEDER_GATEWAY_URL: &str = "https://feeder.alpha-sepolia.starknet.io/feeder_gateway";

pub(crate) async fn fetch_contract_class(feeder_gateway_url: &str, class_hash_hex: &str) -> ContractClass {
    let response = Client::new()
        .get(format!("{feeder_gateway_url}/get_class_by_hash"))
        .query(&[("classHash", class_hash_hex)])
        .send()
        .await
        .expect("Failed to fetch class from feeder gateway")
        .error_for_status()
        .expect("Feeder gateway returned an error");
    let body = response.text().await.expect("Failed to read class response");

    if let Ok(class) = serde_json::from_str::<CoreFlattenedSierraClass>(&body) {
        return ContractClass::Sierra(Arc::new(class.into()));
    }

    let class = serde_json::from_str::<LegacyContractClass>(&body).expect("Failed to parse class response");
    let compressed = class.compress().expect("Failed to compress legacy class response");
    ContractClass::Legacy(Arc::new(compressed.into()))
}

pub(crate) async fn fetch_sierra_class(feeder_gateway_url: &str, class_hash_hex: &str) -> Arc<FlattenedSierraClass> {
    let class = fetch_contract_class(feeder_gateway_url, class_hash_hex).await;
    let ContractClass::Sierra(class) = class else { panic!("Expected Sierra contract class") };
    class
}
