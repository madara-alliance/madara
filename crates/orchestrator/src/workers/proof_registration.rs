use crate::workers::Worker;
use async_trait::async_trait;
use std::error::Error;

pub struct ProofRegistrationWorker;

#[async_trait]
impl Worker for ProofRegistrationWorker {
    /// 1. Fetch all blocks with a successful proving job run
    /// 2. Group blocks that have the same proof
    /// 3. For each group, create a proof registration job with from and to block in metadata
    async fn run_worker(&self) -> Result<(), Box<dyn Error>> {
        todo!()
    }
}
