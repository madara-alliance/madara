use std::fmt;
use std::time::Duration;

use derive_more::FromStr;
use serde::{Deserialize, Serialize};
use url::Url;

use mp_utils::parsers::{parse_duration, parse_url};

#[derive(Clone, Copy, Debug, FromStr, Deserialize, Serialize)]
pub enum MadaraSettlementLayer {
    Eth,
    Starknet,
}

impl fmt::Display for MadaraSettlementLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MadaraSettlementLayer::Eth => write!(f, "ETH"),
            MadaraSettlementLayer::Starknet => write!(f, "STARKNET"),
        }
    }
}

#[derive(Clone, Debug, clap::Args, Deserialize, Serialize)]
pub struct L1SyncParams {
    /// Disable L1 sync. This is handy when:
    /// - You are running the full-node or gateay presets (sync-only node), but you do not have access to an L1 RPC node. You only want to sync
    ///   L2 blocks and serve the RPC and/or (feeder-)gateway protocols.
    /// - You are running a devnet, in which case this field is activated by default if you do not provide an `--l1-endpoint <RPC url>`, for convenience.
    /// - You are running a production chain that is intended not to settle on an L1 chain.
    ///
    /// In general, it is relatevely harmless to disable L1 sync when running sync-only nodes in chains that produce blocks very frequently.
    /// In more details, at time of writing, disabling L1 sync has the following consequences:
    ///
    /// ## RPC and Feeder Gateway block status
    ///
    /// - RPCs and the feeder gateway will not be able to return transaction and block status `ACCEPTED_ON_L1`.
    ///
    /// ## L1 to L2 messages
    ///
    /// - RPC getMessagesStatus (the RPC responsible to get the L2 transaction hashes and status from an L1 transaction hash) will not work.
    /// - When producing blocks, the node will not be able to produce transactions for L1 to L2 messages, because it can't fetch them.
    /// - L2 to L1 messages will still be able to be produced - although if you are running madara without the orchestrator, they will never reach L1.
    ///
    /// ## Gas prices
    ///
    /// The madara backend tracks the most recent `l1_gas_price` and `l1_blob_gas_price`. When disabling L1 sync, **these fields will be empty**, unless
    /// you use `--l1-gas-price <price>` and `--blob-gas-price <price>` to set them to a constant value. See their documentation for details.
    ///
    /// - When devnet mode is enabled these fields are empty, they will be initialized to zero for convenience.
    /// - When producing blocks, the block headers will be initialized with most recent value of these fields. If they are empty, **they will be initialized
    ///   with zeroes**.
    /// - When querying the pre-confirmed block, or when executing on top of it, and there is not currently one present, the RPC specification
    ///   requires us to return a "fake" pre-confirmed block with no transactions. In this case, the header of this *fake* block will be initialized
    ///   with most recent value of these fields. If they are empty, they will instead use the same value as in the most recent confirmed block header, and,
    ///   if there is not yet any confirmed block in the database - pre-genesis state - they will be initialized to zero.
    ///   This has an impact on:
    ///   - All RPCs returning blocks when asking for the pre-confirmed block.
    ///   - RPC execution endpoints, such as estimate_fee/estimate_message_fee/simulate_transaction.
    ///   - Transaction validation (rpc, but also gateway), in particular the gas prices used for fee checking.
    ///
    /// This last point is particularily important when the chain produces blocks very sparsely, as the fee estimations may be much more accurate when l1 sync
    /// is enabled.
    #[clap(env = "MADARA_SYNC_L1_DISABLED", long, alias = "no-l1-sync", conflicts_with = "l1_endpoint")]
    pub l1_sync_disabled: bool,

    /// The L1 rpc endpoint url for state verification. Use with `--settlement-layer STARKNET` to tell madara that this RPC is
    /// a Starknet RPC url rather than an Ethereum one.
    #[clap(env = "MADARA_L1_ENDPOINT", long, value_parser = parse_url, value_name = "ETHEREUM/STARKNET RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// Fix the gas price. If the gas price is fixed it won't fetch the fee history from the ethereum.
    /// When using Ethereum as settlement layer, this price is in WEI. When using Starknet as settlement layer, this price in in FRI.
    #[clap(env = "MADARA_L1_GAS_PRICE", long)]
    pub l1_gas_price: Option<u64>,

    /// Fix the blob gas price. If the gas price is fixed it won't fetch the fee history from the ethereum.
    /// When using Ethereum as settlement layer, this price is in WEI. When using Starknet as settlement layer, this price in in FRI.
    #[clap(env = "MADARA_DATA_GAS_PRICE", long)]
    pub blob_gas_price: Option<u64>,

    // Gas prices! Here are the main cases to consider:
    // - Full-node / gateway: strk_per_eth quote is optional. We can do without it, but if it's provided prefer that.
    //   It is only used when there is no current pre-confirmed block: executing on the latest state will use the most recent quote when available.
    //   When it is not available, it will use the parent block's most recent quote. When the parent block does not exist (genesis does not exist yet!),
    //   it will initialize the quote with zeroes.
    // - Devnet: If strk_per_eth quote is not set, we set it to zero
    /// Fix the eth <-> strk rate. If the strk rate is fixed it won't fetch eth <-> strk price from the oracle.
    #[clap(env = "MADARA_STRK_PER_ETH", long)]
    pub strk_per_eth: Option<f64>,

    /// Oracle API url.
    #[clap(env = "MADARA_ORACLE_URL", long)]
    pub oracle_url: Option<Url>,

    /// Oracle API key.
    #[clap(env = "MADARA_ORACLE_API_KEY", long)]
    pub oracle_api_key: Option<String>,

    /// Polling time for the gas price worker.
    #[clap(
		env = "MADARA_GAS_PRICE_POLL",
        long,
        default_value = "10s",
        value_parser = parse_duration,
    )]
    pub gas_price_poll: Duration,

    /// The settlement layer type. This specifies the type of API `--l1-endpoint <RPC url>` uses: Ethereum RPC, or Starknet RPC.
    /// The usual cases for this is:
    /// - When settling on Ethereum, which is the case for Starknet Mainnet, we are usually running an L2 - because Ethrereum is an L1 and doesn't settle on any other chain.
    /// - When settling on Starknet, we are usually running an L3; because Starknet is already an L2 because it settles on Ethereum.
    ///
    /// This language can be quite confusing at times, because it doesn't really matter for madara whether the settlement layer itself settles on anything else. As
    /// a simplification, all of the madara node documentation will usually use the term "L1" when talking about the *settlement layer* ("the layer above") and L2 when
    /// talking about the current chain.
    /// For even more confusion, you can actually run a madara chain as a standalone L1 using `--no-l1-sync`.
    #[clap(
        env = "MADARA_SETTLEMENT_LAYER",
        long,
        default_value_t = MadaraSettlementLayer::Eth,
    )]
    pub settlement_layer: MadaraSettlementLayer,

    /// Minimum number of block confirmations required before an L1 to L2 message can be processed.
    /// This ensures messages are only processed after they have sufficient confirmations on the settlement layer.
    #[clap(env = "MADARA_L1_MSG_MIN_CONFIRMATIONS", long, default_value = "10")]
    pub l1_msg_min_confirmations: u64,
}
