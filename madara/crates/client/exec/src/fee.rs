use crate::{ExecutionContext, ExecutionResult};
use mc_db::MadaraStorageRead;
use starknet_api::block::{FeeType, GasPriceVector};

impl<D: MadaraStorageRead> ExecutionContext<D> {
    pub fn execution_result_to_fee_estimate(&self, executions_result: &ExecutionResult) -> mp_rpc::FeeEstimate {
        let GasPriceVector { l1_gas_price, l1_data_gas_price, .. } =
            self.block_context.block_info().gas_prices.gas_price_vector(&executions_result.fee_type);
        let l1_gas_price = l1_gas_price.get().0;
        let l1_data_gas_price = l1_data_gas_price.get().0;

        let data_gas_consumed: u128 = executions_result.execution_info.receipt.da_gas.l1_data_gas.0.into();
        let data_gas_fee = data_gas_consumed.saturating_mul(l1_data_gas_price);
        let gas_consumed =
            executions_result.execution_info.receipt.fee.0.saturating_sub(data_gas_fee) / l1_gas_price.max(1);
        let minimal_gas_consumed = executions_result.minimal_l1_gas.unwrap_or_default().l1_gas.0;
        let minimal_data_gas_consumed = executions_result.minimal_l1_gas.unwrap_or_default().l1_data_gas.0;
        let gas_consumed = gas_consumed.max(minimal_gas_consumed.into());
        let data_gas_consumed = data_gas_consumed.max(minimal_data_gas_consumed.into());
        let overall_fee = gas_consumed
            .saturating_mul(l1_gas_price)
            .saturating_add(data_gas_consumed.saturating_mul(l1_data_gas_price));

        let unit = match executions_result.fee_type {
            FeeType::Eth => mp_rpc::PriceUnit::Wei,
            FeeType::Strk => mp_rpc::PriceUnit::Fri,
        };
        mp_rpc::FeeEstimate {
            gas_consumed: gas_consumed.into(),
            gas_price: l1_gas_price.into(),
            data_gas_consumed: data_gas_consumed.into(),
            data_gas_price: l1_data_gas_price.into(),
            overall_fee: overall_fee.into(),
            unit,
        }
    }
}
