pub mod chain_details;
pub mod collections;
pub mod env_utils;
pub mod http_client;
pub mod layer;
pub mod metrics;
pub mod test_utils;

use alloy_primitives::Address;
use std::str::FromStr;

/// Evaluate `$x:expr` and if not true return `Err($y:expr)`.
///
/// Used as `ensure!(expression_to_ensure, expression_to_return_on_false)`.
#[macro_export]
macro_rules! ensure {
    ($x:expr, $y:expr $(,)?) => {{
        if !$x {
            return Err($y);
        }
    }};
}

pub fn address_try_from_str(hex: &str) -> Result<Address, String> {
    // Check for empty string
    if hex.is_empty() {
        return Err("Empty string is not allowed".to_string());
    }

    // Strip "0x" prefix if present
    let cleaned = hex.strip_prefix("0x").unwrap_or(hex);

    // Pad with leading zeros to ensure it's 40 characters long
    let hexstring = format!("0x{:0>40}", cleaned);

    // Attempt to parse the address
    Address::from_str(&hexstring).map_err(|e| format!("Hexstring to Address conversion failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_try_from_str_valid() {
        // Test with a valid full address
        let address = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e";
        let result = address_try_from_str(address);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Address::from_str(address).unwrap());
    }

    #[test]
    fn test_address_try_from_str_without_prefix() {
        // Test without 0x prefix
        let address = "742d35Cc6634C0532925a3b844Bc454e4438f44e";
        let result = address_try_from_str(address);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Address::from_str(&format!("0x{}", address)).unwrap());
    }

    #[test]
    fn test_address_try_from_str_with_padding() {
        // Test with a short address that needs padding
        let short_address = "0xabc123";
        let expected = Address::from_str("0x0000000000000000000000000000000000abc123").unwrap();
        let result = address_try_from_str(short_address);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_address_try_from_str_empty() {
        // Test with an empty string
        let result = address_try_from_str("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty string is not allowed"));
    }

    #[test]
    fn test_address_try_from_str_invalid() {
        // Test with invalid hex characters
        let result = address_try_from_str("0xZZZ");
        assert!(result.is_err());

        // Test with invalid length (too long)
        let too_long = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e00";
        let result = address_try_from_str(too_long);
        assert!(result.is_err());
    }
}
