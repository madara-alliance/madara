use crate::workers::Worker;
use async_trait::async_trait;

pub struct SnosWorker;

#[async_trait]
impl Worker for SnosWorker {
    /// 1. Fetch the latest completed block from the Starknet chain
    /// 2. Fetch the last block that had a SNOS job run.
    /// 3. Create SNOS run jobs for all the remaining blocks
    async fn run_worker(&self) {
        todo!()
    }
}
