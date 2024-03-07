use color_eyre::Result;

pub fn get_env_var(key: &str) -> Result<String> {
    std::env::var(key).map_err(|e| e.into())
}

pub fn get_env_var_or_panic(key: &str) -> String {
    get_env_var(key).unwrap_or_else(|e| panic!("Failed to get env var {}: {}", key, e))
}

pub fn get_env_var_or_default(key: &str, default: &str) -> String {
    get_env_var(key).unwrap_or(default.to_string())
}
