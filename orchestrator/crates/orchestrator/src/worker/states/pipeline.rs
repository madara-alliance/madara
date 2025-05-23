use crate::core::config::Config;
use crate::worker::states::error::StateHandlerError;
use crate::worker::states::handlers::l3_proof_creation::L3ProofCreationHandler;
use crate::worker::states::handlers::snos::SNOSStateHandler;
use crate::worker::states::handlers::snos_verification::SNOSVerificationStateHandler;
use crate::worker::states::types::config::PipelineConfig;
use crate::worker::states::StateHandler;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use super::handlers::l2_data_submission::L2DataSubmissionHandler;
use super::handlers::l2_data_submission_verification::L2DataSubmissionVerificationHandler;
use super::handlers::l2_proof_creation::L2ProofCreationHandler;
use super::handlers::l2_proof_verification::L2ProofVerificationHandler;
use super::handlers::l2_state_update::L2StateUpdateHandler;
use super::handlers::l2_state_update_verification::L2StateUpdateVerificationHandler;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Eq, Hash, PartialEq)]
pub enum State {
    SNOSProcessing,
    SNOSVerification,
    L3ProofCreation,
    L2ProofCreationProcessing,
    L2ProofCreationVerification,
    L2DataSubmissionProcessing,
    L2DataSubmissionVerification,
    L2StateUpdateProcessing,
    L2StateUpdateVerification,
}

impl State {
    pub fn all_states() -> Vec<State> {
        vec![
            State::SNOSProcessing,
            State::SNOSVerification,
            State::L3ProofCreation,
            State::L2ProofCreationProcessing,
            State::L2ProofCreationVerification,
            State::L2DataSubmissionProcessing,
            State::L2DataSubmissionVerification,
            State::L2StateUpdateProcessing,
            State::L2StateUpdateVerification,
        ]
    }
}

impl Default for State {
    fn default() -> Self {
        State::SNOSProcessing
    }
}

// State Pipeline with dependency management
pub struct StatePipeline {
    config: Arc<Config>,
    states: Vec<State>,
    dependencies: Vec<PipelineConfig>,
    handlers: HashMap<State, Box<dyn StateHandler>>,
}

impl StatePipeline {
    async fn initialize_handlers(config: Arc<Config>) -> Result<HashMap<State, Box<dyn StateHandler>>, StateHandlerError> {
        let mut handlers: HashMap<State, Box<dyn StateHandler>> = HashMap::new();
        
        // Initialize handlers with async new()
        handlers.insert(State::SNOSProcessing, Box::new(SNOSStateHandler::new(config.clone()).await));
        handlers.insert(State::SNOSVerification, Box::new(SNOSVerificationStateHandler::new(config.clone()).await));
        handlers.insert(State::L3ProofCreation, Box::new(L3ProofCreationHandler::new(config.clone()).await));
        handlers.insert(State::L2ProofCreationProcessing, Box::new(L2ProofCreationHandler::new(config.clone()).await));
        handlers.insert(State::L2ProofCreationVerification, Box::new(L2ProofVerificationHandler::new(config.clone()).await));
        handlers.insert(State::L2DataSubmissionProcessing, Box::new(L2DataSubmissionHandler::new(config.clone()).await));
        handlers.insert(State::L2DataSubmissionVerification, Box::new(L2DataSubmissionVerificationHandler::new(config.clone()).await));
        handlers.insert(State::L2StateUpdateProcessing, Box::new(L2StateUpdateHandler::new(config.clone()).await));
        handlers.insert(State::L2StateUpdateVerification, Box::new(L2StateUpdateVerificationHandler::new(config.clone()).await));
        
        Ok(handlers)
    }

    fn initialize_dependency() -> Vec<PipelineConfig> {
        vec![
            PipelineConfig {
                state: State::SNOSProcessing,
                depends_on: vec![]
            },
            PipelineConfig {
                state: State::SNOSVerification,
                depends_on: vec![State::SNOSProcessing]
            },
            PipelineConfig {
                state: State::L2ProofCreationProcessing,
                depends_on: vec![State::SNOSVerification]
            },
            PipelineConfig {
                state: State::L2ProofCreationVerification,
                depends_on: vec![State::L2ProofCreationProcessing]
            },
            PipelineConfig {
                state: State::L2DataSubmissionProcessing,
                depends_on: vec![State::L2ProofCreationVerification]
            },
            PipelineConfig {
                state: State::L2DataSubmissionVerification,
                depends_on: vec![State::L2DataSubmissionProcessing]
            },
            PipelineConfig {
                state: State::L2StateUpdateProcessing,
                depends_on: vec![State::L2DataSubmissionVerification]
            },
            PipelineConfig {
                state: State::L2StateUpdateVerification,
                depends_on: vec![State::L2StateUpdateProcessing]
            }
        ]
    }
    /// load the state pipeline
    /// Note: In the Future, this code will be moved to a config file to load the pipeline
    pub async fn load_l3(config: Arc<Config>) -> Result<Self, StateHandlerError> {
        Ok(StatePipeline {
            config: config.clone(),
            states: State::all_states(),
            dependencies: Self::initialize_dependency(),
            handlers: Self::initialize_handlers(config).await?,
        })
    }

    pub fn get_handler(&self, state: &State) -> Result<&Box<dyn StateHandler>, StateHandlerError> {
        self.handlers.get(state).ok_or_else(|| {
            StateHandlerError::InvalidStateHandler(format!("No handler found for state {:?}", state))
        })
    }

    pub fn add_state(&mut self, state: State, depends_on: Vec<State>) {
        self.states.push(state.clone());
        self.add_dependency(PipelineConfig {
            state,
            depends_on
        });
    }

    pub fn add_dependency(&mut self, dependency: PipelineConfig) {
        self.dependencies.push(dependency);
    }

    /// Process a state transition
    pub async fn process_transition(
        &self,
        job_id: Uuid,
        from: &State,
        to: &State,
    ) -> Result<StateProduct, StateHandlerError> {
        // Validate the transition
        self.validate_transition(from, to)?;

        // Get the handler for the target state
        let handler = self.get_handler(to)?;

        // Validate input state
        handler.validate_input(job_id).await?;

        // Process the state
        let result = handler.process(job_id).await?;

        // Validate the output state
        self.validate_state_product(to, &result)?;

        Ok(result)
    }

    /// Validate if a state transition is valid
    pub fn validate_transition(&self, from: &State, to: &State) -> Result<(), StateHandlerError> {
        // Check if the transition is defined in dependencies
        let valid = self.dependencies.iter().any(|dep| {
            dep.state == *to && dep.depends_on.contains(from)
        });

        if !valid {
            return Err(StateHandlerError::InvalidStateHandler(
                format!("Invalid transition from {:?} to {:?}", from, to)
            ));
        }

        Ok(())
    }

    /// Validate if a state product matches the expected type for a state
    pub fn validate_state_product(&self, state: &State, product: &StateProduct) -> Result<(), StateHandlerError> {
        // Get the expected output type for this state
        let expected_type = match state {
            State::SNOSProcessing => StateProduct::SNOSProcessing(Default::default()),
            State::SNOSVerification => StateProduct::SNOSVerification(Default::default()),
            State::L3ProofCreation => StateProduct::L3ProofCreation(Default::default()),
            State::L2ProofCreationProcessing => StateProduct::L2ProofCreationProcessing(Default::default()),
            State::L2ProofCreationVerification => StateProduct::L2ProofCreationVerification {
                proof_data: Default::default(),
                is_valid: true,
                verification_timestamp: 0,
            },
            State::L2DataSubmissionProcessing => StateProduct::L2DataSubmissionProcessing(Default::default()),
            State::L2DataSubmissionVerification => StateProduct::L2DataSubmissionVerification {
                submission_data: Default::default(),
                is_valid: true,
                verification_timestamp: 0,
            },
            State::L2StateUpdateProcessing => StateProduct::L2StateUpdateProcessing(Default::default()),
            State::L2StateUpdateVerification => StateProduct::L2StateUpdateVerification {
                state_update: Default::default(),
                is_valid: true,
                verification_timestamp: 0,
            },
        };

        // Check if the product type matches the expected type
        if std::mem::discriminant(&expected_type) != std::mem::discriminant(product) {
            return Err(StateHandlerError::InvalidInput(
                format!("Invalid product type for state {:?}", state)
            ));
        }

        Ok(())
    }

    /// Get the next possible states from a given state
    pub fn get_next_states(&self, current_state: &State) -> Vec<State> {
        self.dependencies
            .iter()
            .filter(|dep| dep.depends_on.contains(current_state))
            .map(|dep| dep.state.clone())
            .collect()
    }

    /// Check if a state is terminal (has no next states)
    pub fn is_terminal_state(&self, state: &State) -> bool {
        self.get_next_states(state).is_empty()
    }

    /// Get all states that can be processed in parallel
    pub fn get_parallel_states(&self, current_state: &State) -> Vec<State> {
        let next_states = self.get_next_states(current_state);
        let mut parallel_states = vec![];

        for state in next_states {
            // Check if this state can be processed in parallel
            // (i.e., its dependencies are satisfied)
            let dependencies = self.get_dependencies(&state);
            if dependencies.iter().all(|dep| {
                self.is_state_completed(dep)
            }) {
                parallel_states.push(state);
            }
        }

        parallel_states
    }

    /// Get dependencies for a state
    fn get_dependencies(&self, state: &State) -> Vec<State> {
        self.dependencies
            .iter()
            .find(|dep| dep.state == *state)
            .map(|dep| dep.depends_on.clone())
            .unwrap_or_default()
    }

    /// Check if a state is completed
    fn is_state_completed(&self, state: &State) -> bool {
        // This should be implemented based on your state completion criteria
        // For now, we'll assume a state is completed if it's in the pipeline
        self.states.contains(state)
    }
}
