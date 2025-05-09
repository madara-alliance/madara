use crate::MadaraBackend;
use mp_rpc::EmittedEvent;
use starknet_types_core::felt::Felt;

/// EventChannels manages a highly efficient and scalable pub/sub system for events with 16 specific channels
/// plus one "all" channel. This architecture provides several key benefits:
///
/// Benefits:
/// - Selective Subscription: Subscribers can choose between receiving all events or filtering for specific
///   senders, optimizing network and processing resources
/// - Memory Efficiency: The fixed number of channels (16) provides a good balance between granularity
///   and memory overhead
/// - Predictable Routing: The XOR-based hash function ensures consistent and fast mapping of sender
///   addresses to channels
///
/// Events are distributed based on the sender's address in the event, where each sender address
/// is mapped to one of the 16 specific channels using a simple XOR-based hash function.
/// Subscribers can choose to either receive all events or only events from specific senders
/// by subscribing to the corresponding channel.
pub struct EventChannels {
    /// Broadcast channel that receives all events regardless of their sender's address
    all_channels: tokio::sync::broadcast::Sender<EmittedEvent>,
    /// Array of 16 broadcast channels, each handling events from a subset of sender addresses
    /// The target channel for an event is determined by the sender's address mapping
    specific_channels: [tokio::sync::broadcast::Sender<EmittedEvent>; 16],
}

impl EventChannels {
    /// Creates a new EventChannels instance with the specified buffer capacity for each channel.
    /// Each channel (both all_channels and specific channels) will be able to buffer up to
    /// `capacity` events before older events are dropped.
    ///
    /// # Arguments
    /// * `capacity` - The maximum number of events that can be buffered in each channel
    ///
    /// # Returns
    /// A new EventChannels instance with initialized broadcast channels
    pub fn new(capacity: usize) -> Self {
        let (all_channels, _) = tokio::sync::broadcast::channel(capacity);

        let mut specific_channels = Vec::with_capacity(16);
        for _ in 0..16 {
            let (sender, _) = tokio::sync::broadcast::channel(capacity);
            specific_channels.push(sender);
        }

        Self { all_channels, specific_channels: specific_channels.try_into().unwrap() }
    }

    /// Subscribes to events based on an optional sender address filter
    ///
    /// # Arguments
    /// * `from_address` - Optional sender address to filter events:
    ///   * If `Some(address)`, subscribes only to events from senders whose addresses map
    ///     to the same channel as the provided address (address % 16)
    ///   * If `None`, subscribes to all events regardless of sender address
    ///
    /// # Returns
    /// A broadcast::Receiver that will receive either:
    /// * All events (if from_address is None)
    /// * Only events from senders whose addresses map to the same channel as the provided address
    ///
    /// # Warning
    /// This method only provides a coarse filtering mechanism based on address mapping.
    /// You will still need to implement additional filtering in your receiver logic because:
    /// * Multiple sender addresses map to the same channel
    /// * You may want to match the exact sender address rather than just its channel mapping
    ///
    /// # Implementation Details
    /// When a specific address is provided, the method:
    /// 1. Calculates the channel index using the sender's address
    /// 2. Subscribes to the corresponding specific channel
    ///
    /// This means you'll receive events from all senders whose addresses map to the same channel
    pub fn subscribe(&self, from_address: Option<Felt>) -> tokio::sync::broadcast::Receiver<EmittedEvent> {
        match from_address {
            Some(address) => {
                let channel_index = self.calculate_channel_index(&address);
                self.specific_channels[channel_index].subscribe()
            }
            None => self.all_channels.subscribe(),
        }
    }

    /// Publishes an event to both the all_channels and the specific channel determined by the sender's address.
    /// The event will only be sent to channels that have active subscribers.
    ///
    /// # Arguments
    /// * `event` - The event to publish, containing the sender's address that determines the target specific channel
    ///
    /// # Returns
    /// * `Ok(usize)` - The sum of the number of subscribers that received the event across both channels
    /// * `Ok(0)` - If no subscribers exist in any channel
    /// * `Err` - If the event couldn't be sent
    pub fn publish(
        &self,
        event: EmittedEvent,
    ) -> Result<usize, Box<tokio::sync::broadcast::error::SendError<EmittedEvent>>> {
        let channel_index = self.calculate_channel_index(&event.event.from_address);
        let specific_channel = &self.specific_channels[channel_index];

        let mut total = 0;
        if self.all_channels.receiver_count() > 0 {
            total += self.all_channels.send(event.clone())?;
        }
        if specific_channel.receiver_count() > 0 {
            total += specific_channel.send(event)?;
        }
        Ok(total)
    }

    pub fn receiver_count(&self) -> usize {
        self.all_channels.receiver_count() + self.specific_channels.iter().map(|c| c.receiver_count()).sum::<usize>()
    }

    /// Calculates the target channel index for a given sender's address
    ///
    /// # Arguments
    /// * `address` - The Felt address of the event sender to calculate the channel index for
    ///
    /// # Returns
    /// A channel index between 0 and 15, calculated by XORing the two highest limbs of the address
    /// and taking the lowest 4 bits of the result.
    ///
    /// # Implementation Details
    /// Rather than using the last byte of the address, this function:
    /// 1. Gets the raw limbs representation of the address
    /// 2. XORs limbs[0] and limbs[1] (the two lowest limbs)
    /// 3. Uses the lowest 4 bits of the XOR result to determine the channel
    ///
    /// This provides a balanced distribution of addresses across channels by
    /// incorporating entropy from the address
    fn calculate_channel_index(&self, address: &Felt) -> usize {
        let limbs = address.to_raw();
        let hash = limbs[0] ^ limbs[1];
        (hash & 0x0f) as usize
    }
}

impl MadaraBackend {
    #[tracing::instrument(skip(self), fields(module = "EventsChannel"))]
    pub fn subscribe_events(&self, from_address: Option<Felt>) -> tokio::sync::broadcast::Receiver<EmittedEvent> {
        self.watch_events.subscribe(from_address)
    }
}
