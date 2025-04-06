use crate::error::event::EventSystemResult;
use omniqueue::Delivery;

/// MessageParser - Trait to parse the message from the queue
/// This trait is used to parse the message from the queue
/// and convert it into the required format for the worker
pub trait MessageParser {
    fn parse_message(message: &Delivery) -> EventSystemResult<Box<Self>>;
}
