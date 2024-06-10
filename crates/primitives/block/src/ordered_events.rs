use starknet_api::transaction::Event;

#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
/// Starknet OrderEvents definition
pub struct OrderedEvents {
    /// the index of the transaction in the block
    pub index: u128,
    /// The events of the transaction
    pub events: Vec<Event>,
}

impl OrderedEvents {
    /// Creates a new OrderedEvents
    pub fn new(index: u128, events: Vec<Event>) -> Self {
        Self { index, events }
    }

    pub fn index(&self) -> u128 {
        self.index
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }
}
