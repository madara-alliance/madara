use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use mp_chain_config::ChainConfig;

use crate::{notification::ExExNotifications, ExExEvent};

/// Captures the context that an `ExEx` has access to.
pub struct ExExContext {
    /// The chain config
    pub chain_config: Arc<ChainConfig>,

    /// Channel used to send [`ExExEvent`]s to the rest of the node.
    ///
    /// # Important
    ///
    /// The exex should emit a `FinishedHeight` whenever a processed block is safe to prune.
    /// Additionally, the exex can pre-emptively emit a `FinishedHeight` event to specify what
    /// blocks to receive notifications for.
    pub events: UnboundedSender<ExExEvent>,

    /// Channel to receive [`ExExNotification`]s.
    ///
    /// # Important
    ///
    /// Once an [`ExExNotification`] is sent over the channel, it is
    /// considered delivered by the node.
    pub notifications: ExExNotifications,
}
