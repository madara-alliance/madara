use anyhow::{anyhow, bail, ensure, Context};
use serde_yaml::Value;
use starknet_types_core::felt::Felt;

use std::time::Duration;

use url::Url;

/// Parses a "key=value" string & returns a [(String, Value)] tuple.
pub fn parse_key_value_yaml(s: &str) -> anyhow::Result<(String, Value)> {
    let mut parts = s.splitn(2, '=');
    let key = parts.next().ok_or_else(|| anyhow!("Missing key in key-value pair"))?.trim();
    let value = parts.next().ok_or_else(|| anyhow!("Missing value in key-value pair"))?.trim();

    ensure!(!key.trim().is_empty(), "Key cannot be empty");

    // If the value starts with "0x", treat it as a string (to avoid parsing Felt values as numbers)
    let value = if value.starts_with("0x") { Value::String(value.to_string()) } else { serde_yaml::from_str(value)? };

    Ok((key.to_string(), value))
}

/// Parse a string URL & returns it as [Url].
pub fn parse_url(s: &str) -> Result<Url, url::ParseError> {
    s.parse()
}

/// Parses a string duration & return it as [Duration].
pub fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    let split_index = s.find(|c: char| !c.is_ascii_digit()).ok_or_else(|| anyhow!("Invalid duration format: {}", s))?;

    let (value_str, suffix) = s.split_at(split_index);
    let value: u64 = value_str.parse().map_err(|_| anyhow!("Invalid duration value: {}", value_str))?;

    match suffix.trim() {
        "ms" => Ok(Duration::from_millis(value)),
        "s" => Ok(Duration::from_secs(value)),
        "min" => Ok(Duration::from_secs(value * 60)),
        "h" => Ok(Duration::from_secs(value * 60 * 60)),
        _ => bail!("Invalid duration suffix: {}. Expected 'ms', 's', 'min' or 'h'.", suffix),
    }
}

pub fn parse_felt(s: &str) -> anyhow::Result<Felt> {
    Felt::from_hex(s).with_context(|| format!("Invalid felt format: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn test_parse_duration() {
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("200ms").unwrap(), Duration::from_millis(200));
        assert_eq!(parse_duration("5min").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("1 min").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("5h").unwrap(), Duration::from_secs(5 * 60 * 60));
        assert_eq!(parse_duration("10 s").unwrap(), Duration::from_secs(10));
        assert!(parse_duration("2x").is_err());
        assert!(parse_duration("200").is_err());
        assert!(parse_duration("ms200").is_err());
        assert!(parse_duration("-5s").is_err());
        assert!(parse_duration("5.5s").is_err());
    }
}
