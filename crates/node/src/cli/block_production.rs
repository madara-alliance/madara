/// Parameters used to config block production.
#[derive(Clone, Debug, clap::Parser)]
pub struct BlockProductionParams {
    /// Disable the block production service.
    /// The block production service is only enabled with the authority (sequencer) mode.
    #[arg(long, alias = "no-block-production")]
    pub block_production_disabled: bool,

    /// Batch size of transactions taken from the Mempool during block production.
    /// Defaults to 128.
    #[arg(long, alias = "block-production-tx-batch-size", default_value_t = 128)]
    pub block_production_tx_batch_size: usize,
}
