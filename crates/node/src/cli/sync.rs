

#[derive(Clone, Debug, clap::Args)]
pub struct SyncParams {
    #[clap(long)]
    pub sync_disabled: bool,

    /// The L1 rpc endpoint url for state verification
    #[clap(long, value_parser = parse_url)]
    pub l1_endpoint: Option<Url>,

    /// The block you want to start syncing from.
    #[clap(long)]
    pub starting_block: Option<u32>,

    /// The network type to connect to.
    #[clap(long, short, default_value = "integration")]
    pub network: NetworkType,

    /// This will invoke sound interpreted from the block hashes.
    #[clap(long)]
    pub sound: bool,

    /// Disable root verification
    #[clap(long)]
    pub disable_root: bool,

    /// Gateway api key to avoid rate limiting (optional)
    #[clap(long)]
    pub gateway_key: Option<String>,

    /// Polling interval, in seconds
    #[clap(long, default_value = "2")]
    pub sync_polling_interval: u64,

    /// Stop sync polling
    #[clap(long, default_value = "false")]
    pub no_sync_polling: bool,

    /// Number of blocks to sync
    #[clap(long)]
    pub n_blocks_to_sync: Option<u64>,

    #[clap(long)]
    pub backup_every_n_blocks: Option<usize>,

    #[clap(long)]
    pub backup_dir: Option<PathBuf>,

    #[clap(long, default_value = "false")]
    pub restore_from_latest_backup: bool,
}