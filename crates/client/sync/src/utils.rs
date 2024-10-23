use starknet_types_core::felt::Felt;

pub fn trim_hash(hash: &Felt) -> String {
    if (*hash == Felt::ZERO) {
        return "0".to_string();
    }
    let hash_str = format!("{:#x}", hash);
    let hash_len = hash_str.len();

    let prefix = &hash_str[..6 + 2];
    let suffix = &hash_str[hash_len - 6..];

    format!("{}...{}", prefix, suffix)
}
