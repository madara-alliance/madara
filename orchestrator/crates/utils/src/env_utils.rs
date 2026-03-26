use std::env::VarError;
use std::path::Path;

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

pub fn get_env_var_optional_or_panic(key: &str) -> Option<String> {
    get_env_var_optional(key).unwrap_or_else(|e| panic!("Failed to get env var {}: {}", key, e))
}

/// Reads a secret from a file path specified by the `{env_var_name}_FILE` environment variable.
///
/// This supports the file-based secret injection pattern used by Kubernetes CSI Secrets Store
/// Driver, where secrets are mounted as files on a tmpfs volume rather than passed as environment
/// variables. This avoids secrets leaking through `/proc/pid/environ`, crash dumps, logs, or
/// `kubectl describe pod`.
///
/// Returns:
/// - `Ok(Some(content))` if the `_FILE` env var is set and the file is readable
/// - `Ok(None)` if the `_FILE` env var is not set
/// - `Err` if the `_FILE` env var is set but the file can't be read or is empty
pub fn resolve_secret_from_file(env_var_name: &str) -> Result<Option<String>, String> {
    let file_env_var = format!("{}_FILE", env_var_name);
    match std::env::var(&file_env_var) {
        Ok(file_path) if !file_path.is_empty() => {
            let path = Path::new(&file_path);
            let content = std::fs::read_to_string(path).map_err(|e| {
                format!("Failed to read secret from file '{}' (set via {}): {}", file_path, file_env_var, e)
            })?;
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                Err(format!("Secret file '{}' (set via {}) is empty", file_path, file_env_var))
            } else {
                tracing::info!(
                    env_var = env_var_name,
                    file_env_var = file_env_var,
                    file_path = file_path,
                    "Loaded secret from file"
                );
                Ok(Some(trimmed))
            }
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_resolve_secret_from_file_reads_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "  my-secret-value\n  ").unwrap();

        let env_var = "TEST_RESOLVE_SECRET_READS";
        let file_env_var = format!("{}_FILE", env_var);
        std::env::set_var(&file_env_var, tmp.path().to_str().unwrap());

        let result = resolve_secret_from_file(env_var).unwrap();
        assert_eq!(result, Some("my-secret-value".to_string()));

        std::env::remove_var(&file_env_var);
    }

    #[test]
    fn test_resolve_secret_from_file_returns_none_when_unset() {
        let result = resolve_secret_from_file("TEST_RESOLVE_SECRET_UNSET").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_secret_from_file_errors_on_empty_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "   \n  ").unwrap();

        let env_var = "TEST_RESOLVE_SECRET_EMPTY";
        let file_env_var = format!("{}_FILE", env_var);
        std::env::set_var(&file_env_var, tmp.path().to_str().unwrap());

        let result = resolve_secret_from_file(env_var);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("is empty"));

        std::env::remove_var(&file_env_var);
    }

    #[test]
    fn test_resolve_secret_from_file_errors_on_missing_file() {
        let env_var = "TEST_RESOLVE_SECRET_MISSING";
        let file_env_var = format!("{}_FILE", env_var);
        std::env::set_var(&file_env_var, "/nonexistent/path/secret.txt");

        let result = resolve_secret_from_file(env_var);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read secret"));

        std::env::remove_var(&file_env_var);
    }
}
