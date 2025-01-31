use starknet_types_core::felt::Felt;

/// Formats a hash as a shortened hexadecimal string with prefix and suffix.
///
/// If the hash string is 12 characters or shorter, returns the full string.
/// Otherwise, returns a string in the format "0xabcd...ef1234" where the middle
/// is replaced with "...".
///
/// # Arguments
/// * `hash` - The Felt hash to format
///
/// # Returns
/// A formatted string representation of the hash
pub fn trim_hash(hash: &Felt) -> String {
    let hash_str = format!("{:#x}", hash);

    if hash_str.len() <= 12 {
        hash_str
    } else {
        let prefix = &hash_str[..6];
        let suffix = &hash_str[hash_str.len() - 6..];
        format!("{}...{}", prefix, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;

    #[rstest]
    #[case(0, "0x0")]
    #[case(30000000000000, "0x1b48...57e000")]
    #[case(12345678123456789, "0x2bdc...0f5915")]
    fn trim_hash_works(#[case] input: u128, #[case] expected: &str) {
        let trimmed = trim_hash(&Felt::from(input));
        assert_eq!(trimmed, expected);
    }
}
