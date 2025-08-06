use crate::{ExecutionContext, ExecutionResult};
use starknet_api::{
    block::{FeeType, GasPriceVector},
    execution_resources::GasVector,
    transaction::fields::Tip,
};

impl ExecutionContext {
    pub fn execution_result_to_fee_estimate_legacy(
        &self,
        executions_result: &ExecutionResult,
        tip: Tip,
    ) -> mp_rpc::v0_7_1::FeeEstimate {
        let gas_price_vector = self.block_context.block_info().gas_prices.gas_price_vector(&executions_result.fee_type);
        let minimal_gas_vector = executions_result.minimal_l1_gas.unwrap_or_default();
        let gas_vector = executions_result.execution_info.receipt.gas;
        let mut gas_vector = GasVector {
            l1_gas: gas_vector.l1_gas.max(minimal_gas_vector.l1_gas),
            l1_data_gas: gas_vector.l1_data_gas.max(minimal_gas_vector.l1_data_gas),
            l2_gas: gas_vector.l2_gas.max(minimal_gas_vector.l2_gas),
        };
        if executions_result.fee_type == FeeType::Eth {
            gas_vector.l1_gas = gas_vector
                .l1_gas
                .checked_add(
                    self.block_context.versioned_constants().sierra_gas_to_l1_gas_amount_round_up(gas_vector.l2_gas),
                )
                .expect("Gas vector overflow");
        }

        let overall_fee = gas_vector.cost(gas_price_vector, tip);

        let unit = match executions_result.fee_type {
            FeeType::Eth => mp_rpc::v0_7_1::PriceUnit::Wei,
            FeeType::Strk => mp_rpc::v0_7_1::PriceUnit::Fri,
        };

        mp_rpc::v0_7_1::FeeEstimate {
            gas_consumed: gas_vector.l1_gas.0.into(),
            gas_price: gas_price_vector.l1_gas_price.get().0.into(),
            data_gas_consumed: gas_vector.l1_data_gas.0.into(),
            data_gas_price: gas_price_vector.l1_data_gas_price.get().0.into(),
            overall_fee: overall_fee.into(),
            unit,
        }
    }

    pub fn execution_result_to_fee_estimate(&self, executions_result: &ExecutionResult) -> mp_rpc::v0_8_1::FeeEstimate {
        let GasPriceVector { l1_gas_price, l1_data_gas_price, l2_gas_price } =
            self.block_context.block_info().gas_prices.gas_price_vector(&executions_result.fee_type);
        let GasVector { l1_gas, l1_data_gas, l2_gas } = executions_result.execution_info.receipt.gas;

        let unit = match executions_result.fee_type {
            FeeType::Eth => mp_rpc::v0_8_1::PriceUnit::Wei,
            FeeType::Strk => mp_rpc::v0_8_1::PriceUnit::Fri,
        };

        mp_rpc::v0_8_1::FeeEstimate {
            l1_gas_consumed: l1_gas.0,
            l1_gas_price: l1_gas_price.get().0,
            l1_data_gas_consumed: l1_data_gas.0,
            l1_data_gas_price: l1_data_gas_price.get().0,
            l2_gas_consumed: l2_gas.0,
            l2_gas_price: l2_gas_price.get().0,
            overall_fee: executions_result.execution_info.receipt.fee.0,
            unit,
        }
    }
}
