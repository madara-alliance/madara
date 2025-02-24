use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::workers::Worker;

pub struct ProofRegistrationWorker;

#[async_trait]
impl Worker for ProofRegistrationWorker {
    /// 1. Fetch all blocks with a successful proving job run
    /// 2. Group blocks that have the same proof
    /// 3. For each group, create a proof registration job with from and to block in metadata
    async fn run_worker(&self, _config: Arc<Config>) -> color_eyre::Result<()> {
        todo!()
    }
}
