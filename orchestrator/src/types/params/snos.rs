use std::path::PathBuf;

use crate::cli::snos::SNOSCliArgs;
use url::Url;

#[derive(Debug, Clone)]
pub struct SNOSParams {
    pub rpc_for_snos: Url,
    pub snos_full_output: bool,
    pub strk_fee_token_address: String,
    pub eth_fee_token_address: String,
    pub versioned_constants_path: Option<PathBuf>,
}

impl From<SNOSCliArgs> for SNOSParams {
    fn from(args: SNOSCliArgs) -> Self {
        Self {
            rpc_for_snos: args.rpc_for_snos,
            snos_full_output: args.snos_full_output,
            strk_fee_token_address: args.strk_fee_token_address,
            eth_fee_token_address: args.eth_fee_token_address,
            versioned_constants_path: args.versioned_constants_path,
        }
    }
}
