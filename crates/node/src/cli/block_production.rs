/// Parameters used to config telemetry.
#[derive(Clone, Debug, clap::Parser)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(long, alias = "no-disabled")]
    pub block_production_disabled: bool,
}
