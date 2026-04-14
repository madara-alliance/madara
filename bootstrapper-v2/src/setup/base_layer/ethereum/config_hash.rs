use crate::config::ConfigHashConfig;
use starknet::core::crypto::compute_hash_on_elements;
use starknet::core::types::Felt;
use thiserror::Error;

/// Default config hash version constant: 'StarknetOsConfig3' encoded as hex
/// 0x537461726b6e65744f73436f6e66696733
pub const DEFAULT_CONFIG_HASH_VERSION: &str = "0x537461726b6e65744f73436f6e66696733";

#[derive(Error, Debug)]
pub enum ConfigHashError {
    #[error("Failed to parse hex value '{value}': {reason}")]
    HexParseError { value: String, reason: String },
}

/// Parameters required for computing the OS config hash
pub struct ConfigHashParams {
    /// Config hash version (defaults to StarknetOsConfig3)
    pub version: Felt,
    /// Chain ID from config
    pub chain_id: Felt,
    /// Madara fee token address on L2
    pub madara_fee_token: Felt,
    /// Optional DA public keys for computing public_keys_hash
    pub da_public_keys: Option<Vec<Felt>>,
}

impl TryFrom<&ConfigHashConfig> for ConfigHashParams {
    type Error = ConfigHashError;

    fn try_from(config: &ConfigHashConfig) -> Result<Self, Self::Error> {
        // version and madara_chain_id support both hex (0x...) and ASCII string formats
        let version = felt_from_string(&config.version)?;
        let chain_id = felt_from_string(&config.madara_chain_id)?;
        let madara_fee_token = felt_from_hex(&config.madara_fee_token)?;

        // Parse DA public keys if present
        let da_public_keys: Option<Vec<Felt>> = if config.da_public_keys.is_empty() {
            None
        } else {
            let keys: Result<Vec<Felt>, _> = config.da_public_keys.iter().map(|k| felt_from_hex(k)).collect();
            Some(keys?)
        };

        Ok(Self { version, chain_id, madara_fee_token, da_public_keys })
    }
}

/// Parses a hex string (with or without 0x prefix) into a Felt
pub fn felt_from_hex(hex_str: &str) -> Result<Felt, ConfigHashError> {
    Felt::from_hex(hex_str)
        .map_err(|e| ConfigHashError::HexParseError { value: hex_str.to_string(), reason: e.to_string() })
}

/// Parses a string into a Felt, supporting both:
/// - Hex strings starting with "0x" (e.g., "0x4d41444152415f4445564e4554")
/// - ASCII strings (e.g., "MADARA_DEVNET") which are converted to their byte representation
pub fn felt_from_string(s: &str) -> Result<Felt, ConfigHashError> {
    if s.starts_with("0x") || s.starts_with("0X") {
        // Parse as hex
        felt_from_hex(s)
    } else {
        // Convert ASCII string to bytes and then to Felt
        // from_bytes_be_slice handles the conversion directly
        Ok(Felt::from_bytes_be_slice(s.as_bytes()))
    }
}

/// Computes the Pedersen hash over the DA public keys array
/// Returns Felt::ZERO if the keys array is empty or None
pub fn compute_public_keys_hash(da_public_keys: &Option<Vec<Felt>>) -> Felt {
    match da_public_keys {
        Some(keys) if !keys.is_empty() => compute_hash_on_elements(keys),
        _ => Felt::ZERO,
    }
}

impl ConfigHashParams {
    /// Creates a new ConfigHashParams with an overridden fee token.
    /// Useful when the deployed L2 fee token differs from the configured one.
    pub fn with_fee_token(self, fee_token: Felt) -> Self {
        Self { madara_fee_token: fee_token, ..self }
    }

    /// Computes the OS config hash using Pedersen hash algorithm
    ///
    /// Formula:
    /// ```text
    /// config_hash = Pedersen::hash_array([version, chain_id, madara_fee_token, public_keys_hash?])
    /// ```
    ///
    /// Where `public_keys_hash` is only included if non-zero
    pub fn compute_os_config_hash(&self) -> Felt {
        let public_keys_hash = compute_public_keys_hash(&self.da_public_keys);

        let mut data = vec![self.version, self.chain_id, self.madara_fee_token];

        if public_keys_hash != Felt::ZERO {
            data.push(public_keys_hash);
        }

        compute_hash_on_elements(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_felt_from_string_hex_format() {
        // Test hex format with 0x prefix
        let result = felt_from_string("0x4d41444152415f4445564e4554").unwrap();
        let expected = Felt::from_hex("0x4d41444152415f4445564e4554").unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_felt_from_string_ascii_format() {
        // Test ASCII string format - should produce same result as hex
        let from_ascii = felt_from_string("MADARA_DEVNET").unwrap();
        let from_hex = felt_from_string("0x4d41444152415f4445564e4554").unwrap();
        assert_eq!(from_ascii, from_hex, "ASCII 'MADARA_DEVNET' should equal hex '0x4d41444152415f4445564e4554'");
    }

    #[test]
    fn test_felt_from_string_version_formats() {
        // Test that StarknetOsConfig3 works in both formats
        let from_hex = felt_from_string(DEFAULT_CONFIG_HASH_VERSION).unwrap();
        let from_ascii = felt_from_string("StarknetOsConfig3").unwrap();
        assert_eq!(from_ascii, from_hex, "ASCII 'StarknetOsConfig3' should equal hex version");
    }

    #[test]
    fn test_compute_public_keys_hash_empty() {
        let result = compute_public_keys_hash(&None);
        assert_eq!(result, Felt::ZERO);

        let result = compute_public_keys_hash(&Some(vec![]));
        assert_eq!(result, Felt::ZERO);
    }

    #[test]
    fn test_compute_public_keys_hash_with_keys() {
        let keys = Some(vec![Felt::from_hex("0x1").unwrap(), Felt::from_hex("0x2").unwrap()]);
        let result = compute_public_keys_hash(&keys);
        assert_ne!(result, Felt::ZERO);
    }

    #[test]
    fn test_compute_os_config_hash_madara_devnet() {
        // Test with MADARA_DEVNET chain_id and empty DA keys
        // Expected config hash: 0x032ee9a5311d00fb7ecb72de6eeeb0352349fa58766e387715342021ac546aca
        let params = ConfigHashParams {
            version: felt_from_hex(DEFAULT_CONFIG_HASH_VERSION).unwrap(),
            chain_id: Felt::from_hex("0x4d41444152415f4445564e4554").unwrap(), // MADARA_DEVNET
            madara_fee_token: Felt::from_hex("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d")
                .unwrap(),
            da_public_keys: None,
        };

        let result = params.compute_os_config_hash();
        let expected = Felt::from_hex("0x032ee9a5311d00fb7ecb72de6eeeb0352349fa58766e387715342021ac546aca").unwrap();
        assert_eq!(result, expected, "Config hash mismatch for MADARA_DEVNET with empty DA keys");
    }

    #[test]
    fn test_compute_os_config_hash_with_public_keys() {
        // Test with MADARA_DEVNET chain_id and 3 DA public keys
        // Verifies that the hash is different from the empty DA keys case
        let params = ConfigHashParams {
            version: felt_from_hex(DEFAULT_CONFIG_HASH_VERSION).unwrap(),
            chain_id: Felt::from_hex("0x4d41444152415f4445564e4554").unwrap(), // MADARA_DEVNET
            madara_fee_token: Felt::from_hex("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d")
                .unwrap(),
            da_public_keys: Some(vec![
                Felt::from_hex("0x0582ef3a536a8dc8e84ce3369c4fc4f3482bb2fdc9f2df8bd75a5ab9a6d4e63e").unwrap(),
                Felt::from_hex("0x00960d67e261edb4e9490ccfe1192e1deca1346569576beb264e25441a38b306").unwrap(),
                Felt::from_hex("0x1c75bb410d184c5ae4fb55930396fc73b71399976ec1fbb371712494c23f80d").unwrap(),
            ]),
        };

        let result = params.compute_os_config_hash();
        assert_ne!(result, Felt::ZERO);

        // The hash with DA keys should be different from the hash without DA keys
        let empty_da_expected =
            Felt::from_hex("0x032ee9a5311d00fb7ecb72de6eeeb0352349fa58766e387715342021ac546aca").unwrap();
        assert_ne!(result, empty_da_expected, "Config hash with DA keys should differ from empty DA keys case");
    }
}
