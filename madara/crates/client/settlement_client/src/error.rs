use crate::eth::error::EthereumClientError;
use crate::starknet::error::StarknetClientError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SettlementClientError {
    #[error("Ethereum client error: {0}")]
    Ethereum(EthereumClientError),

    #[error("Starknet client error: {0}")]
    Starknet(StarknetClientError),

    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    #[error("Value exceeds Felt max value for field: {0}")]
    ValueExceedsFeltRange(&'static str),

    #[error("Invalid log: {0}")]
    InvalidLog(String),

    #[error("Invalid contract: {0}")]
    InvalidContract(String),

    #[error("Conversion error: {0}")]
    ConversionError(String),

    #[error("Invalid event: {0}")]
    InvalidEvent(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Messaging synchronization error: {0}")]
    MessagingSync(String),

    #[error("Stream processing error: {0}")]
    StreamProcessing(String),

    #[error("Mempool error: {0}")]
    Mempool(String),

    #[error("Gas price error: {0}")]
    GasPrice(String),

    #[error("Time calculation error: {0}")]
    TimeCalculation(String),

    #[error("Price oracle error: {0}")]
    PriceOracle(String),

    #[error("Block number error: {0}")]
    BlockNumber(String),

    #[error("State update error: {0}")]
    StateUpdate(String),

    #[error("State initialization error: {0}")]
    StateInitialization(String),

    #[error("State event listener error: {0}")]
    StateEventListener(String),

    #[error("{0}")]
    Other(String),
}

// 1. Ensure EthereumClientError can be converted to SettlementClientError
impl From<EthereumClientError> for SettlementClientError {
    fn from(err: EthereumClientError) -> Self {
        SettlementClientError::Ethereum(err)
    }
}

// 2. Ensure StarknetClientError can be converted to SettlementClientError
impl From<StarknetClientError> for SettlementClientError {
    fn from(err: StarknetClientError) -> Self {
        SettlementClientError::Starknet(err)
    }
}
