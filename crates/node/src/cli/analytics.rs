use clap::Args;
use serde::Deserialize;
use mp_utils::parsers::parse_url;
use tracing::Level;
use url::Url;

/// Parameters used to config analytics.
#[derive(Debug, Clone, Args, Deserialize)]
#[serde(default)]
pub struct AnalyticsParams {
    /// Name of the service.
    #[arg(env = "MADARA_ANALYTICS_SERVICE_NAME", long, alias = "analytics", default_value = "madara_analytics")]
    pub analytics_service_name: String,

    /// Log level of the service.
    #[arg(env = "MADARA_ANALYTICS_LOG_LEVEL", long, default_value = "info")]
    #[serde(with = "LevelDef")]
    pub analytics_log_level: Level,

    /// Endpoint of the analytics server.
    #[arg(env = "MADARA_ANALYTICS_COLLECTION_ENDPOINT", long, value_parser = parse_url, default_value = None)]
    pub analytics_collection_endpoint: Option<Url>,
}

impl Default for AnalyticsParams {
    fn default() -> Self {
        Self {
            analytics_service_name: "madara_analytics".to_string(),
            analytics_log_level: Level::INFO,
            analytics_collection_endpoint: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(remote = "Level")]
enum LevelDef {
    TRACE,
    DEBUG,
    INFO,
    WARN,
    ERROR,
}

impl From<LevelDef> for Level {
    fn from(value: LevelDef) -> Self {
        match value {
            LevelDef::TRACE => Self::TRACE,
            LevelDef::DEBUG => Self::DEBUG,
            LevelDef::INFO => Self::INFO,
            LevelDef::WARN => Self::WARN,
            LevelDef::ERROR => Self::ERROR,
        }
    }
}
