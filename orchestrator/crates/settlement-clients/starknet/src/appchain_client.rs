use crate::interfaces::core_contract::CoreContract;
use crate::interfaces::messaging::Messaging;
use crate::interfaces::operator::Operator;
use starknet::core::types::Felt;

use crate::utils::LocalWalletSignerMiddleware;

pub struct StarknetCoreContractClient {
    operator: Operator,
    messaging: Messaging,
    core_contract: CoreContract,
}

impl StarknetCoreContractClient {
    pub fn new(address: Felt, signer: LocalWalletSignerMiddleware) -> Self {
        Self {
            operator: Operator::new(address, signer.clone()),
            messaging: Messaging::new(address, signer.clone()),
            core_contract: CoreContract::new(address, signer),
        }
    }
}

impl AsRef<Operator> for StarknetCoreContractClient {
    fn as_ref(&self) -> &Operator {
        &self.operator
    }
}

impl AsRef<Messaging> for StarknetCoreContractClient {
    fn as_ref(&self) -> &Messaging {
        &self.messaging
    }
}

impl AsRef<CoreContract> for StarknetCoreContractClient {
    fn as_ref(&self) -> &CoreContract {
        &self.core_contract
    }
}
