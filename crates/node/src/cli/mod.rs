pub mod block_production;
pub mod db;
pub mod l1;
pub mod prometheus;
pub mod rpc;
pub mod sync;
pub mod telemetry;

use self::block_production::BlockProductionParams;
use crate::cli::l1::L1SyncParams;
pub use db::*;
pub use prometheus::*;
pub use rpc::*;
pub use sync::*;
pub use telemetry::*;

#[derive(Clone, Debug, clap::Parser)]
pub struct RunCmd {
    /// The human-readable name for this node.
    /// It is used as the network node name.
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
    pub l1_sync_params: L1SyncParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub telemetry_params: TelemetryParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub prometheus_params: PrometheusParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub rpc_params: RpcParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub block_production_params: BlockProductionParams,

    /// Enable authority mode: the node will run as a sequencer and try and produce its own blocks.
    #[arg(long)]
    pub authority: bool,

    /// Run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,
}

impl RunCmd {
    pub async fn node_name_or_provide(&mut self) -> &str {
        if self.name.is_none() {
            let name = dc_sync::utility::get_random_pokemon_name().await.unwrap_or_else(|e| {
                log::warn!("Failed to get random pokemon name: {}", e);
                "deoxys".to_string()
            });

            self.name = Some(name);
        }
        self.name.as_ref().unwrap()
    }

    pub async fn network(&mut self) -> &str {
        if self.sync_params.network == NetworkType::Integration {
            "Integration"
        } else if self.sync_params.network == NetworkType::Test {
            "Testnet"
        } else {
            "Mainnet"
        }
    }
}
