#[derive(Clone, Debug, clap::Args)]
pub struct P2pParams {
    /// Enable the p2p service.
    #[arg(env = "MADARA_P2P", long)]
    pub p2p: bool,

    /// Port for peer-to-peer. By default, it will ask the os for an unused port.
    #[arg(env = "MADARA_P2P_PORT", long)]
    pub p2p_port: Option<u16>,
}
