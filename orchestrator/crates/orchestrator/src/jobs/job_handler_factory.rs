use mockall::automock;

#[automock]
pub mod factory {
    use std::sync::Arc;

    #[allow(unused_imports)]
    use mockall::automock;

    use crate::jobs::types::JobType;
    use crate::jobs::{da_job, proving_job, snos_job, state_update_job, Job};

    /// To get the job handler
    //         +-------------------+
    //         |                   |
    //         |  Arc<Box<dyn Job>>|
    //         |                   |
    //          +--------+----------+
    //                   |
    //                  |    +----------------+
    //                  |    |                |
    //                  +--->| Box<dyn Job>   |
    //                  |    |                |
    //                  |    +----------------+
    //                  |             |
    //                  |             |
    //          +-------v-------+     |
    //          |               |     |
    //          | Closure 1     |     |
    //          |               |     |
    //          +---------------+     |
    //                                |
    //          +---------------+     |
    //          |               |     |
    //          | Closure x     |     |
    //          |               |     |
    //          +---------------+     |
    //                                |
    //                                |
    //                                v
    //                         +--------------+
    //                         |              |
    //                         | dyn Job      |
    //                         | (job_handler)|
    //                         |              |
    //                         +--------------+
    /// We are using Arc so that we can call the Arc::clone while testing that will point
    /// to the same Box<dyn Job>. So when we are mocking the behaviour :
    ///
    /// - We create the MockJob
    /// - We return this mocked job whenever a function calls `get_job_handler`
    /// - Making it an Arc allows us to return the same MockJob in multiple calls to
    ///   `get_job_handler`. This is needed because `MockJob` doesn't implement Clone
    pub async fn get_job_handler(job_type: &JobType) -> Arc<Box<dyn Job>> {
        // Original implementation
        let job: Box<dyn Job> = match job_type {
            JobType::DataSubmission => Box::new(da_job::DaJob),
            JobType::SnosRun => Box::new(snos_job::SnosJob),
            JobType::ProofCreation => Box::new(proving_job::ProvingJob),
            JobType::StateTransition => Box::new(state_update_job::StateUpdateJob),
            _ => unimplemented!("Job type not implemented yet."),
        };

        Arc::new(job)
    }
}
