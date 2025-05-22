use omniqueue::Delivery;

#[derive(Debug)]
pub enum MessageType {
    Message(Delivery),
    NoMessage,
}
