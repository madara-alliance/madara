pub mod db;
pub mod prometheus;
pub mod rpc;
pub mod sync;
pub mod telemetry;

use std::fmt;

pub use db::*;
pub use prometheus::*;
pub use rpc::*;
pub use sync::*;
pub use telemetry::*;

pub enum CliError {
    UsageError,
    InternalError,
}

pub trait ResultExt<T, E> {
    fn or_internal_error<C: fmt::Display>(self, context: C) -> Result<T, CliError>;
}

impl<T, E: Into<anyhow::Error>> ResultExt<T, E> for Result<T, E> {
    #[inline]
    fn or_internal_error<C: fmt::Display>(self, context: C) -> Result<T, CliError> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                log::error!("{}: {:#}", context, E::into(err));
                Err(CliError::InternalError)
            }
        }
    }
}

#[derive(Clone, Debug, clap::Parser)]
pub struct RunCmd {
    /// The human-readable name for this node.
    /// It's used as network node name.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub db_params: DbParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub sync_params: SyncParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub telemetry_params: TelemetryParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub prometheus_params: PrometheusParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub rpc_params: RpcParams,

    /// Run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,
}

impl RunCmd {
    pub fn name(&mut self) -> String {
        self.name
            .get_or_insert_with(|| {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(mc_sync::utility::get_random_pokemon_name())
                    .unwrap_or_else(|e| {
                        log::warn!("Failed to get random pokemon name: {}", e);
                        "deoxys".to_string()
                    })
            })
            .clone()
    }
}
