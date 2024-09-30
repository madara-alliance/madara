use anyhow::{anyhow, bail};

use std::time::Duration;

use url::Url;

/// Parses a "key=value" string & returns a [(String, String)] tuple.
pub fn parse_key_value(s: &str) -> anyhow::Result<(String, String)> {
    let mut parts = s.splitn(2, '=');
    let key = parts.next().ok_or_else(|| anyhow::anyhow!("Invalid key-value pair"))?;
    let value = parts.next().ok_or_else(|| anyhow::anyhow!("Invalid key-value pair"))?;
    Ok((key.to_string(), value.to_string()))
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
        _ => bail!("Invalid duration suffix: {}. Expected 'ms', 's', or 'min'.", suffix),
    }
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
        assert_eq!(parse_duration("10 s").unwrap(), Duration::from_secs(10));
        assert!(parse_duration("2x").is_err());
        assert!(parse_duration("200").is_err());
        assert!(parse_duration("5h").is_err());
        assert!(parse_duration("ms200").is_err());
        assert!(parse_duration("-5s").is_err());
        assert!(parse_duration("5.5s").is_err());
    }
}
