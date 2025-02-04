use crate::{model, FromModelError};
use mp_receipt::{Event, EventWithTransactionHash};

impl From<EventWithTransactionHash> for model::Event {
    fn from(value: EventWithTransactionHash) -> Self {
        Self {
            transaction_hash: Some(value.transaction_hash.into()),
            from_address: Some(value.event.from_address.into()),
            keys: value.event.keys.into_iter().map(Into::into).collect(),
            data: value.event.data.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<model::Event> for EventWithTransactionHash {
    type Error = FromModelError;
    fn try_from(value: model::Event) -> Result<Self, Self::Error> {
        Ok(Self {
            transaction_hash: value
                .transaction_hash
                .ok_or(FromModelError::missing_field("Event::transaction_hash"))?
                .into(),
            event: Event {
                from_address: value.from_address.ok_or(FromModelError::missing_field("Event::from_address"))?.into(),
                keys: value.keys.into_iter().map(Into::into).collect(),
                data: value.data.into_iter().map(Into::into).collect(),
            },
        })
    }
}
