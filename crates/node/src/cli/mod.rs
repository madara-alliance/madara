pub mod rpc;
pub mod telemetry;
pub mod prometheus;
pub mod sync;
pub mod db;

use std::fmt;

pub use rpc::*;
pub use telemetry::*;
pub use prometheus::*;
pub use sync::*;
pub use db::*;

pub enum CliError {
    UsageError,
    InternalError,
}

pub trait ResultExt {
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

#[derive(Clone, Debug, clap::Args)]
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
    fn deoxys_environment(&mut self) {
        // Set the blockchain network to 'starknet'
        cmd.base.shared_params.chain = Some("starknet".to_string());
        cmd.base.shared_params.base_path.get_or_insert_with(|| PathBuf::from("/tmp/deoxys"));
    
        // Define telemetry endpoints at starknodes.com
        cmd.base.telemetry_params.telemetry_endpoints = vec![("wss://starknodes.com/submit/".to_string(), 0)];
    
        // Enables manual sealing for custom block production
        cmd.base.no_grandpa = true;
        cmd.sealing = Some(Sealing::Manual);
    }

    fn execute(mut self) -> CliResult<()> {
        #[cfg(feature = "tui")]
        todo!();
        // {
        //     deoxys_tui::modify_substrate_sources();
        //     if cli.run.tui {
        //         std::thread::spawn(move || {
        //             tokio::runtime::Runtime::new()
        //                 .unwrap()
        //                 .block_on(async { deoxys_tui::run("/tmp/deoxys").await.unwrap() });
        //             std::process::exit(0)
        //         });
        //     }
        // }

        // Assign a random pokemon name at each startup
        self.name.get_or_insert_with(|| {
            tokio::runtime::Runtime::new().unwrap().block_on(mc_sync::utility::get_random_pokemon_name()).unwrap_or_else(
                |e| {
                    log::warn!("Failed to get random pokemon name: {}", e);
                    "deoxys".to_string()
                },
            )
        });

        self.deoxys_environment();

        // If --no-telemetry is not set, set the telemetry endpoints to starknodes.com
        // TODO(merge): telemetry
        // if !cli.run.base.telemetry_params.no_telemetry {
        //     cli.run.base.telemetry_params.telemetry_endpoints =
        // vec![("wss://starknodes.com/submit/".to_string(), 0)]; }

        // TODO: verify that the l1_endpoint is valid
        let l1_endpoint = if let Some(url) = cli.run.l1_endpoint {
            url
        } else {
            log::error!("Missing required --l1-endpoint argument. The Online documentation is available here: https://deoxys-docs.kasar.io");
            return Ok(());
        };

        let starting_block = cli.run.starting_block;
        let mut fetch_block_config = cli.run.network.block_fetch_config();
        fetch_block_config.sound = cli.run.sound;
        fetch_block_config.verify = !cli.run.disable_root;
        fetch_block_config.api_key = cli.run.gateway_key.clone();
        fetch_block_config.sync_polling_interval =
            if cli.run.no_sync_polling { None } else { Some(Duration::from_secs(cli.run.sync_polling_interval)) };
        fetch_block_config.n_blocks_to_sync = cli.run.n_blocks_to_sync;
        // unique set of static OnceCell configuration
        set_config(&fetch_block_config);

        let genesis_block = fetch_apply_genesis_block(fetch_block_config.clone()).await.unwrap();

        service::new_full(
            config,
            sealing,
            l1_endpoint,
            fetch_block_config,
            genesis_block,
            starting_block,
            cli.run.backup_every_n_blocks,
            cli.run.backup_dir,
            cli.run.restore_from_latest_backup,
        )

        Ok(())
    }
}
