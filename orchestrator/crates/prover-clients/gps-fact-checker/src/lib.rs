use alloy::network::{Ethereum, EthereumWallet};
use alloy::primitives::{Address, FixedBytes, B256};
use alloy::providers::fillers::{
    BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller,
};
use alloy::providers::{Identity, ProviderBuilder, RootProvider};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use std::str::FromStr;
use std::sync::Arc;
use url::Url;

// Minimal on-chain verifier ABI shared by the real Starkware GPS verifier and our
// `MockGpsVerifier`. `isValid` is on both; `registerFact` is only on the mock
// (the real verifier proves and registers in one step server-side). Declaring
// both here is safe: runtime calls are dispatched by selector against the
// actual on-chain bytecode, so a `registerFact` call against the real verifier
// would simply revert (and we never make such a call).
sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    interface GpsVerifier {
        function isValid(bytes32 fact) external view returns (bool);
        function registerFact(bytes32 fact) external;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FactCheckerError {
    #[error("Fact registry call failed: {0}")]
    InvalidFact(#[source] alloy::contract::Error),
    #[error("registerFact submission failed: {0}")]
    RegisterSend(#[source] alloy::contract::Error),
    #[error("Waiting for registerFact receipt failed: {0}")]
    RegisterReceipt(#[source] alloy::providers::PendingTransactionError),
    #[error("registerFact tx {0:#x} reverted")]
    RegisterReverted(B256),
    #[error("registerFact tx {0:#x} mined but isValid(fact) still false — wrong contract at verifier_address?")]
    RegisterNotEffective(B256),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettlementLayer {
    Ethereum,
    Starknet,
}

impl FromStr for SettlementLayer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ethereum" => Ok(SettlementLayer::Ethereum),
            "starknet" => Ok(SettlementLayer::Starknet),
            _ => Err(format!("Unknown settlement layer: {}", s)),
        }
    }
}

pub struct FactChecker {
    gps_verifier: Option<GpsVerifier::GpsVerifierInstance<ReadProvider>>,
    settlement_layer: SettlementLayer,
}

/// Read-only provider produced by `ProviderBuilder::new().connect_http(..)`.
pub type ReadProvider = FillProvider<
    JoinFill<Identity, JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>>,
    RootProvider<Ethereum>,
    Ethereum,
>;

impl FactChecker {
    pub fn new(rpc_url: Url, gps_verifier_contract_address: String, settlement_layer: String) -> Self {
        let settlement_layer = SettlementLayer::from_str(&settlement_layer).expect("Invalid settlement layer");

        match settlement_layer {
            SettlementLayer::Ethereum => {
                let provider = ProviderBuilder::new().connect_http(rpc_url);
                let gps_verifier = GpsVerifier::new(
                    Address::from_str(gps_verifier_contract_address.as_str())
                        .expect("Invalid GPS verifier contract address"),
                    provider,
                );
                Self { gps_verifier: Some(gps_verifier), settlement_layer }
            }
            SettlementLayer::Starknet => Self { gps_verifier: None, settlement_layer },
        }
    }

    pub async fn is_valid(&self, fact: &B256) -> Result<bool, FactCheckerError> {
        match self.settlement_layer {
            SettlementLayer::Ethereum => {
                let gps_verifier =
                    self.gps_verifier.as_ref().expect("Fact registry should be initialized for Ethereum");
                gps_verifier.isValid(*fact).call().await.map_err(FactCheckerError::InvalidFact)
            }
            SettlementLayer::Starknet => {
                // TODO:L3 Implement actual Starknet fact checking
                // For now, return true as a mock implementation
                Ok(true)
            }
        }
    }
}

// =============================================================================
// FactRegistrar — write path for the mock verifier
// =============================================================================

/// Wallet-backed provider used to sign `registerFact` txs.
pub type WalletProvider = FillProvider<
    JoinFill<
        JoinFill<Identity, JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>>,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Ethereum>,
    Ethereum,
>;

/// Registers facts on an on-chain verifier that implements the `registerFact(bytes32)`
/// interface (e.g. our `MockGpsVerifier`). Intended for mocknet/dev use only — never
/// deploy against a real verifier.
///
/// Idempotent: every write goes through `isValid` first, so repeated calls with the
/// same fact do not double-spend gas.
pub struct FactRegistrar {
    verifier_address: Address,
    provider: Arc<WalletProvider>,
}

impl FactRegistrar {
    pub fn new(rpc_url: Url, signer: PrivateKeySigner, verifier_address: Address) -> Self {
        let wallet = EthereumWallet::from(signer);
        let provider = Arc::new(ProviderBuilder::new().wallet(wallet).connect_http(rpc_url));
        Self { verifier_address, provider }
    }

    fn contract(&self) -> GpsVerifier::GpsVerifierInstance<Arc<WalletProvider>> {
        GpsVerifier::new(self.verifier_address, self.provider.clone())
    }

    /// `isValid(fact)` on the configured verifier.
    pub async fn is_registered(&self, fact: [u8; 32]) -> Result<bool, FactCheckerError> {
        self.contract().isValid(FixedBytes::<32>::from(fact)).call().await.map_err(FactCheckerError::InvalidFact)
    }

    /// Ensure `fact` is registered on-chain, idempotently.
    ///
    /// - If it is already registered, returns `B256::ZERO` (no tx was sent).
    /// - Otherwise submits `registerFact(fact)`, waits for the receipt, and
    ///   cross-checks via `isValid` before returning the tx hash.
    pub async fn ensure_registered(&self, fact: [u8; 32]) -> Result<B256, FactCheckerError> {
        if self.is_registered(fact).await? {
            return Ok(B256::ZERO);
        }
        let pending = self
            .contract()
            .registerFact(FixedBytes::<32>::from(fact))
            .send()
            .await
            .map_err(FactCheckerError::RegisterSend)?;
        let receipt = pending.get_receipt().await.map_err(FactCheckerError::RegisterReceipt)?;
        if !receipt.status() {
            return Err(FactCheckerError::RegisterReverted(receipt.transaction_hash));
        }
        // Catches misconfigured verifier_address / wrong-contract-at-address bugs.
        if !self.is_registered(fact).await? {
            return Err(FactCheckerError::RegisterNotEffective(receipt.transaction_hash));
        }
        Ok(receipt.transaction_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settlement_layer_from_str() {
        assert_eq!(SettlementLayer::from_str("ethereum").unwrap(), SettlementLayer::Ethereum);
        assert_eq!(SettlementLayer::from_str("starknet").unwrap(), SettlementLayer::Starknet);
        assert!(SettlementLayer::from_str("invalid").is_err());
    }
}
