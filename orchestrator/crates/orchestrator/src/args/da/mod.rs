// Comment out the external crate import
// use ethereum_da_client::EthereumDaValidatedArgs;

pub mod ethereum;

// Define a local struct for now
#[derive(Debug, Clone)]
pub struct EthereumDaValidatedArgs {
    pub rpc_url: String,
}

#[derive(Debug, Clone)]
pub enum DaValidatedArgs {
    Ethereum(EthereumDaValidatedArgs),
}
