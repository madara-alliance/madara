// @Paradex contract schema
// Contract: Paraclear

use crate::types::ContractAddress;
use starknet_types_core::felt::Felt;

#[derive(Clone, Debug, PartialEq)]
pub struct ByteArray {
    pub data: Vec<[u8; 31]>,
    pub pending_word: Felt,
    pub pending_word_len: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeeWithCapRequest {
    pub fee: Felt,
    pub fee_cap: Felt,
    pub fee_floor: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeeWithCapRequestV2 {
    pub fee: Felt,
    pub fee_cap: Felt,
    pub fee_floor: Felt,
    pub fee_token: ContractAddress,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderV3 {
    pub account: ContractAddress,
    pub market: Felt,
    pub side: Felt,
    pub orderType: Felt,
    pub size: Felt,
    pub price: Felt,
    pub signature_timestamp: Felt,
    pub is_reduce_only: bool,
    pub order_category: OrderCategory,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TradeRequestV3 {
    pub id: Felt,
    pub size: Felt,
    pub price: Felt,
    pub traded_at: Felt,
    pub maker_order: OrderV3,
    pub taker_order: OrderV3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EthAddress {
    pub address: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenAssetBalance {
    pub token_address: ContractAddress,
    pub amount: Felt,
    pub prev: ContractAddress,
    pub next: ContractAddress,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualAssetBalance {
    pub market: Felt,
    pub amount: Felt,
    pub cost: Felt,
    pub cached_funding: Felt,
    pub prev: Felt,
    pub next: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualMarginParams {
    pub imf_base: Felt,
    pub imf_factor: Felt,
    pub mmf_factor: Felt,
    pub imf_shift: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualAsset {
    pub market: Felt,
    pub base_asset: Felt,
    pub quote_asset: Felt,
    pub tick_size: Felt,
    pub margin_params: PerpetualMarginParams,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualAssetBalanceDisplay {
    pub market: Felt,
    pub amount: Felt,
    pub cost: Felt,
    pub cached_funding: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarketFeeConfigRequest {
    pub maker_api: Felt,
    pub taker_api: Felt,
    pub maker_rpi: Felt,
    pub taker_rpi: Felt,
    pub maker_interactive: Felt,
    pub taker_interactive: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualOptionAsset {
    pub market: Felt,
    pub base_asset: Felt,
    pub quote_asset: Felt,
    pub tick_size: Felt,
    pub option_type: Felt,
    pub strike: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualOptionMarginParams {
    pub premium_multiplier: Felt,
    pub long_itm: Felt,
    pub short_itm: Felt,
    pub short_otm: Felt,
    pub short_put_cap: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerpetualOptionCrossMarginParams {
    pub imf: PerpetualOptionMarginParams,
    pub mmf: PerpetualOptionMarginParams,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TokenAsset {
    pub initial_weight: Felt,
    pub maintenance_weight: Felt,
    pub conversion_weight: Felt,
    pub tick_size: Felt,
    pub token_address: ContractAddress,
    pub token_name: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeeRate {
    pub exists: Felt,
    pub maker: Felt,
    pub taker: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccountReferral {
    pub referrer: ContractAddress,
    pub fee_commission: Felt,
    pub fee_discount: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BridgeAction {
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OrderCategory {
    Unspecified,
    API,
    RPI,
    Interactive,
    Dynamic(FeeWithCapRequest),
    DynamicWithToken(FeeWithCapRequestV2),
}

#[derive(Clone, Debug, PartialEq)]
pub enum MarginMethodology {
    CrossMargin,
    PortfolioMargin,
}
