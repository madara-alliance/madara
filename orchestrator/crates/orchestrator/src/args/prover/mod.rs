// Comment out external crate imports
// use atlantic_service::AtlanticValidatedArgs;
// use sharp_service::SharpValidatedArgs;

use url::Url;

pub mod atlantic;
pub mod sharp;



#[derive(Debug, Clone)]
pub enum ProverValidatedArgs {
    Sharp(sharp::SharpValidatedArgs),
    Atlantic(atlantic::AtlanticValidatedArgs),
}
