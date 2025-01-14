use std::env::VarError;

pub fn get_env_var(key: &str) -> Result<String, VarError> {
    std::env::var(key)
}

pub fn get_env_var_or_panic(key: &str) -> String {
    get_env_var(key).unwrap_or_else(|e| panic!("Failed to get env var {}: {}", key, e))
}

pub fn get_env_var_or_default(key: &str, default: &str) -> String {
    get_env_var(key).unwrap_or(default.to_string())
}

pub fn get_env_var_optional(key: &str) -> Result<Option<String>, VarError> {
    match get_env_var(key) {
        // if value is empty string, return None
        Ok(s) if s.is_empty() => Ok(None),
        Ok(s) => Ok(Some(s)),
        Err(VarError::NotPresent) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn get_env_car_optional_or_panic(key: &str) -> Option<String> {
    get_env_var_optional(key).unwrap_or_else(|e| panic!("Failed to get env var {}: {}", key, e))
}
