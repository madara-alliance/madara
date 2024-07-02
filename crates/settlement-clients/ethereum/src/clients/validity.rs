use std::sync::Arc;

use alloy::{network::Ethereum, primitives::Address, transports::http::Http};

use crate::clients::interfaces::validity_interface::StarknetValidityContract;
use crate::types::LocalWalletSignerMiddleware;

// TODO: should be moved to Zaun:
// https://github.com/keep-starknet-strange/zaun

/// Client to interact with a Starknet core contract running in `Validity` mode
pub struct StarknetValidityContractClient {
    core_contract: StarknetValidityContract::StarknetValidityContractInstance<
        Http<reqwest::Client>,
        Arc<LocalWalletSignerMiddleware>,
        Ethereum,
    >,
}

impl StarknetValidityContractClient {
    pub fn new(address: Address, client: Arc<LocalWalletSignerMiddleware>) -> Self {
        Self { core_contract: StarknetValidityContract::new(address, client.clone()) }
    }
}

impl
    AsRef<
        StarknetValidityContract::StarknetValidityContractInstance<
            Http<reqwest::Client>,
            Arc<LocalWalletSignerMiddleware>,
            Ethereum,
        >,
    > for StarknetValidityContractClient
{
    fn as_ref(
        &self,
    ) -> &StarknetValidityContract::StarknetValidityContractInstance<
        Http<reqwest::Client>,
        Arc<LocalWalletSignerMiddleware>,
        Ethereum,
    > {
        &self.core_contract
    }
}
