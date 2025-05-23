use crate::worker::states::StateHandler;

pub mod product;
pub mod config;

// pub fn get_state_handler(state: String) -> Result<Box<dyn StateHandler>, StateHandlerError> {
//     match state.as_str() {
//         "SnosProcessing" => Ok(Box::new(SnosStateHandler)),
//         "L2Proving" => Ok(Box::new(L2ProvingStateHandler)),
//         "L3Proving" => Ok(Box::new(L3ProvingStateHandler)),
//         "L2ProvingReg" => Ok(Box::new(L2ProvingRegStateHandler)),
//         "DA" => Ok(Box::new(DAStateHandler)),
//         "StateUpdate" => Ok(Box::new(StateUpdateHandler)),
//         _ => Err(StateHandlerError::InvalidStateHandler(state)),
//     }
// }