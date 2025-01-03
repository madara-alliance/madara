use clap::Args;

/// Parameters used to config telemetry.
#[derive(Debug, Clone, Args)]
pub struct TelemetryParams {
    /// Enable connecting to the Madara telemetry server.
    #[arg(env = "MADARA_TELEMETRY", long, alias = "telemetry")]
    pub telemetry: bool,

    /// The URL of the telemetry server.
    /// Pass this flag multiple times specify multiple telemetry endpoints.
    /// Verbosity levels range from 0-9, with 0 denoting
    /// the least verbosity.
    /// Expected format is 'URL VERBOSITY', e.g. `--telemetry-url 'wss://foo/bar 0'`.
    #[arg(
		env = "MADARA_TELEMETRY_URL",
		long = "telemetry-url",
		value_name = "URL VERBOSITY",
		value_parser = parse_telemetry_endpoints,
		default_value = "wss://starknodes.com/submit 0",
	)]
    pub telemetry_endpoints: Vec<(String, u8)>,
}

#[derive(Debug, thiserror::Error)]
enum TelemetryParsingError {
    #[error("verbosity must be an int")]
    VerbosityParsingError(std::num::ParseIntError),
}

fn parse_telemetry_endpoints(s: &str) -> Result<(String, u8), TelemetryParsingError> {
    match s.find(' ') {
        None => Ok((s.to_string(), 1)),
        Some(pos) => {
            let url = s[..pos].to_string();
            let verbosity = s[pos + 1..].parse().map_err(TelemetryParsingError::VerbosityParsingError)?;
            Ok((url, verbosity))
        }
    }
}
