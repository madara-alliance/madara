use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashMap;

/// Interpolate environment variables in a string
/// Supports ${VAR} and ${VAR:-default} syntax
pub fn interpolate_env_vars(input: &str) -> Result<String> {
    let re = Regex::new(r"\$\{([^}:]+)(?::-(.*?))?\}").unwrap();
    let mut result = input.to_string();
    let mut missing_vars = Vec::new();

    for cap in re.captures_iter(input) {
        let full_match = cap.get(0).unwrap().as_str();
        let var_name = cap.get(1).unwrap().as_str();
        let default_value = cap.get(2).map(|m| m.as_str());

        let value = match std::env::var(var_name) {
            Ok(val) => val,
            Err(_) => {
                if let Some(default) = default_value {
                    default.to_string()
                } else {
                    missing_vars.push(var_name.to_string());
                    continue;
                }
            }
        };

        result = result.replace(full_match, &value);
    }

    if !missing_vars.is_empty() {
        anyhow::bail!("Missing required environment variables: {}", missing_vars.join(", "));
    }

    Ok(result)
}

/// Interpolate environment variables in YAML content
pub fn interpolate_yaml_content(yaml_content: &str) -> Result<String> {
    let lines: Vec<String> = yaml_content
        .lines()
        .map(|line| {
            // Don't interpolate comments
            if line.trim().starts_with('#') {
                return Ok(line.to_string());
            }

            interpolate_env_vars(line)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(lines.join("\n"))
}

/// Load environment variables from a .env file
pub fn load_env_file(path: &std::path::Path) -> Result<HashMap<String, String>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read .env file: {}", path.display()))?;

    let mut vars = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse KEY=VALUE
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
            vars.insert(key, value);
        }
    }

    Ok(vars)
}

/// Set environment variables from a hashmap
pub fn set_env_vars(vars: HashMap<String, String>) {
    for (key, value) in vars {
        std::env::set_var(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_simple() {
        std::env::set_var("TEST_VAR", "test_value");
        let result = interpolate_env_vars("Hello ${TEST_VAR}").unwrap();
        assert_eq!(result, "Hello test_value");
        std::env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_interpolate_with_default() {
        std::env::remove_var("MISSING_VAR");
        let result = interpolate_env_vars("Value: ${MISSING_VAR:-default}").unwrap();
        assert_eq!(result, "Value: default");
    }

    #[test]
    fn test_interpolate_missing_required() {
        std::env::remove_var("REQUIRED_VAR");
        let result = interpolate_env_vars("Value: ${REQUIRED_VAR}");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("REQUIRED_VAR"));
    }

    #[test]
    fn test_interpolate_multiple() {
        std::env::set_var("VAR1", "value1");
        std::env::set_var("VAR2", "value2");
        let result = interpolate_env_vars("${VAR1} and ${VAR2}").unwrap();
        assert_eq!(result, "value1 and value2");
        std::env::remove_var("VAR1");
        std::env::remove_var("VAR2");
    }

    #[test]
    fn test_yaml_interpolation() {
        std::env::set_var("DB_URL", "mongodb://localhost:27017");
        let yaml = r#"
database:
  url: ${DB_URL}
  # This is a comment with ${SHOULD_NOT_INTERPOLATE}
  name: test
"#;
        let result = interpolate_yaml_content(yaml).unwrap();
        assert!(result.contains("mongodb://localhost:27017"));
        assert!(result.contains("${SHOULD_NOT_INTERPOLATE}")); // Comments not interpolated
        std::env::remove_var("DB_URL");
    }
}
