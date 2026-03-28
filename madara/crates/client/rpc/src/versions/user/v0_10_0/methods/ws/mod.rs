pub mod lib;
pub mod starknet_unsubscribe;
// v0.10.0 reuses the v0.9.0 WS implementations through delegation in `lib.rs`.

#[allow(unused)]
const BLOCK_PAST_LIMIT: u64 = 1024;
#[allow(unused)]
const ADDRESS_FILTER_LIMIT: u64 = 128;

#[derive(PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionItem<T> {
    subscription_id: u64,
    result: T,
}

impl<T> SubscriptionItem<T> {
    pub fn new(subscription_id: jsonrpsee::types::SubscriptionId, result: T) -> Self {
        let subscription_id = match subscription_id {
            jsonrpsee::types::SubscriptionId::Num(id) => id,
            jsonrpsee::types::SubscriptionId::Str(_) => {
                unreachable!("Jsonrpsee middleware has been configured to use u64 subscription ids")
            }
        };

        Self { subscription_id, result }
    }
}
