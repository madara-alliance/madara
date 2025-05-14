use crate::error::event::EventSystemResult;
use crate::error::other::OtherError;
use crate::error::ConsumptionError;
use crate::types::jobs::WorkerTriggerType;
use crate::worker::traits::message::MessageParser;
use color_eyre::eyre::Context;
use omniqueue::Delivery;
use serde::Serialize;
use std::str::FromStr;

#[derive(Debug, Serialize, Clone)]
pub struct WorkerTriggerMessage {
    pub worker: WorkerTriggerType,
}

impl MessageParser for WorkerTriggerMessage {
    fn parse_message(message: &Delivery) -> EventSystemResult<Box<Self>> {
        let payload = message
            .borrow_payload()
            .ok_or_else(|| ConsumptionError::Other(OtherError::from("Empty payload".to_string())))?;
        let message_string = String::from_utf8_lossy(payload).to_string().trim_matches('\"').to_string();
        let trigger_type = WorkerTriggerType::from_str(message_string.as_str())
            .wrap_err("Failed to parse worker trigger type from message")
            .map_err(|e| ConsumptionError::Other(OtherError::from(e)))?;
        Ok(Box::new(Self { worker: trigger_type }))
    }
}
