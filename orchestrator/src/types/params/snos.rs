use crate::cli::snos::SNOSCliArgs;
use url::Url;

#[derive(Debug, Clone)]
pub struct SNOSParams {
    pub rpc_for_snos: Url,
    pub full_output: bool,
}

impl From<SNOSCliArgs> for SNOSParams {
    fn from(args: SNOSCliArgs) -> Self {
        Self { rpc_for_snos: args.rpc_for_snos, full_output: args.full_output }
    }
}
