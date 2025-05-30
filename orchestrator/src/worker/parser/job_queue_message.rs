use crate::error::event::EventSystemResult;
use crate::error::ConsumptionError;
use crate::worker::traits::message::MessageParser;
use omniqueue::Delivery;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobQueueMessage {
    pub id: Uuid,
}

impl MessageParser for JobQueueMessage {
    fn parse_message(message: &Delivery) -> EventSystemResult<Box<Self>> {
        let result = message
            .payload_serde_json::<Self>()
            .map_err(|e| ConsumptionError::PayloadSerdeError(e.to_string()))?
            .ok_or_else(|| ConsumptionError::PayloadSerdeError(String::from("Empty payload")))?;
        Ok(Box::new(result))
    }
}
