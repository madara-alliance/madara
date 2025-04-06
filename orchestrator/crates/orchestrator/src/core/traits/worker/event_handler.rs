use async_trait::async_trait;

/// EventHandlerTrait - Trait for event handler
/// This trait defines the methods that an event handler must implement
/// to be able to handle events in an event system
#[async_trait]
pub trait EventHandlerTrait: Send + Sync {
}