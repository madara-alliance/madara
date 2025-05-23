use crate::core::config::Config;
use crate::core::DatabaseClient;
use crate::types::queue::QueueType;
use crate::worker::states::error::StateHandlerError;
use crate::worker::states::pipeline::{State, StatePipeline};
use crate::worker::states::types::product::StateProduct;
use std::sync::Arc;
use uuid::Uuid;
use tokio::sync::Semaphore;
use std::sync::atomic::{AtomicBool, Ordering};

// Orchestrator that handles parallel processing
pub struct StateOrchestrator {
    config: Arc<Config>,
    pipeline: StatePipeline,
    processing: Arc<AtomicBool>,
    concurrency_limit: Arc<Semaphore>,
}

impl StateOrchestrator {
    pub fn new(config: Arc<Config>) -> Self {
        let concurrency_limit = Arc::new(Semaphore::new(10)); // Adjust based on your needs
        StateOrchestrator {
            config: config.clone(),
            pipeline: StatePipeline::load_l3(config).await.expect("Failed to load pipeline"),
            processing: Arc::new(AtomicBool::new(false)),
            concurrency_limit,
        }
    }

    /// database client - used to persist the state
    fn database(&self) -> &dyn DatabaseClient {
        self.config.database()
    }

    /// Process the job by running the states in parallel
    /// if queue_type is None, it will run all the states in the pipeline
    /// if queue_type is Some, it will run the states in the pipeline that are of the same type
    pub async fn process_job(&self, job_id: Uuid, queue_type: Option<QueueType>) -> Result<(), StateHandlerError> {
        // Prevent concurrent processing of the same job
        if self.processing.load(Ordering::SeqCst) {
            return Err(StateHandlerError::ProcessStateError(
                "Job is already being processed".to_string()
            ));
        }
        self.processing.store(true, Ordering::SeqCst);

        // Get the current state of the job
        let current_state = self.get_current_state(job_id).await?;

        // Get next possible states
        let next_states = self.pipeline.get_next_states(&current_state);

        // Process states in parallel with concurrency limit
        let mut handles = vec![];
        for state in next_states {
            let permit = self.concurrency_limit.clone().acquire_owned().await
                .map_err(|e| StateHandlerError::ProcessStateError(e.to_string()))?;
            
            let handler = self.pipeline.get_handler(&state)?;
            let job_id = job_id;
            
            let handle = tokio::spawn(async move {
                let _permit = permit; // Hold the permit until the task completes
                handler.process(job_id).await
            });
            
            handles.push(handle);
        }

        // Wait for all states to complete
        for handle in handles {
            handle.await
                .map_err(|e| StateHandlerError::ProcessStateError(e.to_string()))?
                .map_err(|e| StateHandlerError::ProcessStateError(e.to_string()))?;
        }

        self.processing.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Get the current state of a job
    async fn get_current_state(&self, job_id: Uuid) -> Result<State, StateHandlerError> {
        // Get the latest state from the database
        let states = self.database()
            .get_job_states(job_id)
            .await
            .map_err(|e| StateHandlerError::ProcessStateError(e.to_string()))?;

        // If no states exist, start with the default state
        if states.is_empty() {
            return Ok(State::default());
        }

        // Get the latest state
        states.last()
            .ok_or_else(|| StateHandlerError::ProcessStateError("No states found".to_string()))
            .map(|state| state.clone())
    }
}