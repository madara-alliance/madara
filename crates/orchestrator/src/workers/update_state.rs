use std::error::Error;

use async_trait::async_trait;

use crate::workers::Worker;

pub struct UpdateStateWorker;

#[async_trait]
impl Worker for UpdateStateWorker {
    /// 1. Fetch the last succesful state update job
    /// 2. Fetch all succesful proving jobs covering blocks after the last state update
    /// 3. Create state updates for all the blocks that don't have a state update job
    async fn run_worker(&self) -> Result<(), Box<dyn Error>> {
        todo!()
    }
}
