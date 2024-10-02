use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use mp_block::MadaraPendingBlock;
use starknet_api::block::BlockNumber;
use tokio::sync::mpsc::Receiver;

/// Notifications sent to an `ExEx`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ExExNotification {
    /// A new block got produced by the Block Production task.
    BlockProduced { block: Box<MadaraPendingBlock>, block_number: BlockNumber },
    /// A new block got synced by the full node.
    BlockSynced { block_number: BlockNumber },
}

/// A stream of [`ExExNotification`]s. The stream will emit notifications for all blocks.
#[derive(Debug)]
pub struct ExExNotifications {
    notifications: Receiver<ExExNotification>,
}

impl ExExNotifications {
    /// Creates a new instance of [`ExExNotifications`].
    pub const fn new(notifications: Receiver<ExExNotification>) -> Self {
        Self { notifications }
    }
}

impl Stream for ExExNotifications {
    type Item = ExExNotification;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().notifications.poll_recv(cx)
    }
}
