use std::path::PathBuf;

#[derive(Clone, Debug, clap::Args)]
pub struct P2pParams {
    /// Enable the p2p service.
    #[arg(env = "MADARA_P2P", long)]
    pub p2p: bool,

    /// Port for peer-to-peer. By default, it will ask the os for an unused port.
    #[arg(env = "MADARA_P2P_PORT", long)]
    pub p2p_port: Option<u16>,

    /// Peer-to-peer identity file. By default, we generate a new one everytime the node starts.
    ///
    /// Use `--p2p-save-identity` with this argument to generate and save the identity file
    /// if it is not present. If the `--p2p-save-identity` argument is not set and the identity file
    /// does not exist, the node will exit with an error.
    ///
    /// Usage example: `--p2p-identity-file identity.json --p2p-save-identity`.
    #[arg(env = "MADARA_P2P_IDENTITY_FILE", long)]
    pub p2p_identity_file: Option<PathBuf>,

    /// Use with `--p2p-identity-file`.
    #[arg(env = "MADARA_P2P_SAVE_IDENTITY", long)]
    pub p2p_save_identity: bool,
}
