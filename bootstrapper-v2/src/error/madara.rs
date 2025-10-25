use starknet::accounts::single_owner::SignError;
use starknet::accounts::{AccountError, AccountFactoryError};
use starknet::core::types::contract::{ComputeClassHashError, JsonError};
use starknet::core::types::FromStrError;
use starknet::signers::local_wallet::SignError as LocalWalletSignError;
use thiserror::Error;

pub type RevertReason = String;
pub type EventName = String;
pub type FilePath = String;

#[derive(Error, Debug)]
pub enum MadaraError {
    #[error("Invalid private key format: {0}")]
    InvalidPrivateKeyFormat(#[from] FromStrError),

    #[error("Failed to create OpenZeppelin account factory: {0}")]
    FailedToCreateOpenZeppelinAccountFactory(#[from] starknet::signers::Infallible),

    #[error("Failed deploying OpenZeppelin account: {0}")]
    FailedToDeployOpenZeppelinAccount(#[from] AccountFactoryError<LocalWalletSignError>),

    #[error("Expected invoke transaction receipt")]
    ExpectedInvokeTransactionReceipt,

    #[error("Failed to open file {1}: {0}")]
    FailedToOpenFile(#[source] std::io::Error, FilePath),

    #[error("Failed to parse file {1}: {0}")]
    FailedToParseFile(#[source] serde_json::Error, FilePath),

    #[error("Failed to compute class hash: {0}")]
    FailedToComputeClassHash(#[from] ComputeClassHashError),

    #[error("Failed to flatten contract artifact: {0}")]
    FailedToFlattenContractArtifact(#[from] JsonError),

    #[error("Failed with starknet.rs account error: {0}")]
    StarknetAccountError(#[from] AccountError<SignError<LocalWalletSignError>>),

    #[error("Madara transaction with tag {1} reverted with reason: {0}")]
    FailedToWaitForTransaction(RevertReason, String),

    #[error("Failed to get event from transaction receipt: {0}")]
    FailedToGetEventFromTransactionReceipt(EventName),

    #[error("Account not initialized - must call init() first")]
    AccountNotInitialized,

    #[error("Class hash not found for contract: {0}")]
    ClassHashNotFound(String),

    #[error("Required field '{0}' not found in base layer addresses")]
    MissingBaseLayerAddress(String),

    #[error("Provider error: {0}")]
    ProviderError(#[from] starknet::providers::ProviderError),

    #[error("ContractsDeployed event data too short, expected 3 addresses")]
    ContractsDeployedEventDataTooShort,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Eth address parse error: {0}")]
    EthAddressParseError(#[from] starknet::core::types::eth_address::FromHexError),

    #[error("File error: {0}")]
    FileError(#[from] crate::utils::FileError),
}
