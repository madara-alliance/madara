use atlantic_service::AtlanticValidatedArgs;
use sharp_service::SharpValidatedArgs;

pub mod atlantic;
pub mod sharp;

#[derive(Debug, Clone)]
pub enum ProverValidatedArgs {
    Sharp(SharpValidatedArgs),
    Atlantic(AtlanticValidatedArgs),
}
