use crypto_bigint::{Encoding, NonZero, U256};
use rand::{rngs::StdRng, Rng, SeedableRng};
use starknet_types_core::felt::Felt;

/// A private key store with zeroing safeguards
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ZeroingPrivateKey {
    #[serde(skip_serializing)]
    private: Felt,
    pub public: Felt,
}

impl ZeroingPrivateKey {
    /// Stores a private key and zeroes the issuing variable
    pub fn new(private: &mut Felt) -> Self {
        let public = starknet_crypto::get_public_key(private);
        let s = Self { private: *private, public };

        core::mem::take(private);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

        s
    }

    // Implementation taken from starknet-signers
    // https://github.com/xJonathanLEI/starknet-rs/blob/1b1071e2c5975c8810c1b05b776aaa58cb172037/starknet-signers/src/key_pair.rs#L113
    pub fn sign(
        &self,
        hash: &Felt,
    ) -> Result<starknet::core::crypto::Signature, starknet::core::crypto::EcdsaSignError> {
        starknet::core::crypto::ecdsa_sign(&self.private, hash).map(Into::into)
    }
}

impl Default for ZeroingPrivateKey {
    // Implementation taken from starknet-signers
    // https://github.com/xJonathanLEI/starknet-rs/blob/1b1071e2c5975c8810c1b05b776aaa58cb172037/starknet-signers/src/key_pair.rs#L38
    fn default() -> Self {
        const PRIME: NonZero<U256> =
            NonZero::from_uint(U256::from_be_hex("0800000000000011000000000000000000000000000000000000000000000001"));

        let mut rng = StdRng::from_entropy();
        let mut buffer = [0u8; 32];
        rng.fill(&mut buffer);

        let random_u256 = U256::from_be_slice(&buffer);
        let private = random_u256.rem(&PRIME);
        let mut private = Felt::from_bytes_be_slice(&private.to_be_bytes());
        let public = starknet_crypto::get_public_key(&private);

        let s = Self { private, public };

        // This makes sure that only the private key in s is leaked out
        // of this function
        core::mem::take(&mut private);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);

        s
    }
}

impl Drop for ZeroingPrivateKey {
    fn drop(&mut self) {
        core::mem::take(&mut self.private);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst)
    }
}

impl core::fmt::Debug for ZeroingPrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZeroingPrivateKey")
            .field("private", &"***" as &dyn core::fmt::Debug)
            .field("public", &self.public)
            .finish()
    }
}

impl TryFrom<String> for ZeroingPrivateKey {
    type Error = starknet::core::types::FromStrError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let s = zeroize::Zeroizing::new(s);
        let private = Felt::from_hex(&s)?;
        let public = starknet_crypto::get_public_key(&private);

        Ok(Self { private, public })
    }
}
