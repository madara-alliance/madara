//! ExEx of Pragma Dispatcher
//! Adds a new TX at the end of each block, dispatching a message through
//! Hyperlane.

use futures::StreamExt;
use mp_exex::{ExExContext, ExExEvent};

pub async fn exex_pragma_dispatch(mut ctx: ExExContext) -> anyhow::Result<()> {
    while let Some(notification) = ctx.notifications.next().await {
        let block_number = notification.closed_block();
        log::info!("👋 Hello from the ExEx (triggered at block #{})", block_number);
        ctx.events.send(ExExEvent::FinishedHeight(block_number))?;
    }
    Ok(())
}
