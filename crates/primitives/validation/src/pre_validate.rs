use starknet_api::core::ChainId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Validation {
    /// Use the transaction hashes from the transaction receipts instead of computing them.
    pub trust_transaction_hashes: bool,
    pub chain_id: ChainId,
}

pub trait PreValidate {
    type Output;
    type ValidationError;

    fn pre_validate(self, validation: &Validation) -> Result<Self::Output, Self::ValidationError>;
}
