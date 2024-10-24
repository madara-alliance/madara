use starknet_types_core::felt::Felt;

pub fn trim_hash(hash: &Felt) -> String {
    let hash_str = format!("{:#x}", hash);

    if hash_str.len() <= 12 {
        hash_str.to_string()
    } else {
        let prefix = &hash_str[..6];
        let suffix = &hash_str[hash_str.len() - 6..];
        format!("{}...{}", prefix, suffix)
    }
}
