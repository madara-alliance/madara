use crate::cron::event_bridge::AWSEventBridgeValidatedArgs;

pub mod event_bridge;

#[derive(Clone, Debug)]
pub enum CronValidatedArgs {
    AWSEventBridge(AWSEventBridgeValidatedArgs),
}
