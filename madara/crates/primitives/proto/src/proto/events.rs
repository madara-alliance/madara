use crate::{model, model_field, CollectInto, FromModelError};
use m_proc_macros::model_describe;
use mp_receipt::{Event, EventWithTransactionHash};

impl From<EventWithTransactionHash> for model::Event {
    fn from(value: EventWithTransactionHash) -> Self {
        Self {
            transaction_hash: Some(value.transaction_hash.into()),
            from_address: Some(value.event.from_address.into()),
            keys: value.event.keys.collect_into(),
            data: value.event.data.collect_into(),
        }
    }
}

impl TryFrom<model::Event> for EventWithTransactionHash {
    type Error = FromModelError;

    #[model_describe(model::Event)]
    fn try_from(value: model::Event) -> Result<Self, Self::Error> {
        Ok(Self {
            transaction_hash: model_field!(value => transaction_hash).into(),
            event: Event {
                from_address: model_field!(value => from_address).into(),
                keys: value.keys.collect_into(),
                data: value.data.collect_into(),
            },
        })
    }
}
