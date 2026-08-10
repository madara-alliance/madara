// @Paradex contract schema
// Contract: AssetsManager

use crate::core::types::ContractAddress;
use starknet_types_core::felt::Felt;

#[derive(Clone, Debug, PartialEq)]
pub struct NamedToken {
    pub token_address: ContractAddress,
    pub token_name: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpotAsset {
    pub market: Felt,
    pub base_token_address: ContractAddress,
    pub base_token_name: Felt,
    pub quote_asset: Felt,
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
pub struct MarketFeeConfigRequest {
    pub maker_api: Felt,
    pub taker_api: Felt,
    pub maker_rpi: Felt,
    pub taker_rpi: Felt,
    pub maker_interactive: Felt,
    pub taker_interactive: Felt,
    pub max_fee_rate: Option<Felt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarketFeeConfig {
    pub exists: bool,
    pub maker_api: Felt,
    pub taker_api: Felt,
    pub maker_rpi: Felt,
    pub taker_rpi: Felt,
    pub maker_interactive: Felt,
    pub taker_interactive: Felt,
    pub max_fee_rate: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeeWithCap {
    pub fee: Felt,
    pub fee_cap: Felt,
    pub fee_floor: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionMarketFeeConfig {
    pub exists: bool,
    pub maker_api: FeeWithCap,
    pub taker_api: FeeWithCap,
    pub maker_rpi: FeeWithCap,
    pub taker_rpi: FeeWithCap,
    pub maker_interactive: FeeWithCap,
    pub taker_interactive: FeeWithCap,
    pub max_fee_rate: FeeWithCap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionMarketFeeConfigRequest {
    pub maker_api: FeeWithCap,
    pub taker_api: FeeWithCap,
    pub maker_rpi: FeeWithCap,
    pub taker_rpi: FeeWithCap,
    pub maker_interactive: FeeWithCap,
    pub taker_interactive: FeeWithCap,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionAsset {
    pub market: Felt,
    pub base_asset: Felt,
    pub quote_asset: Felt,
    pub tick_size: Felt,
    pub option_type: Felt,
    pub strike: Felt,
    pub expiry_time: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionMarginParams {
    pub premium_multiplier: Felt,
    pub long_itm: Felt,
    pub short_itm: Felt,
    pub short_otm: Felt,
    pub short_put_cap: Felt,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionCrossMarginParams {
    pub imf: OptionMarginParams,
    pub mmf: OptionMarginParams,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeeCategory {
    Unspecified,
    API,
    RPI,
    Interactive,
}
