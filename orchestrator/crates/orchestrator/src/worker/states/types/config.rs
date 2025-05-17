use crate::worker::states::pipeline::State;

#[derive(Debug, Clone, Default)]
pub struct PipelineConfig {
    pub state: State,
    pub depends_on: Vec<State>,
}


