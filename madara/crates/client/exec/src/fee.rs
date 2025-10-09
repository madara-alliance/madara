use crate::{Error, ExecutionContext, ExecutionResult};
use anyhow::Context;
use mc_db::MadaraStorageRead;
use starknet_api::{block::FeeType, execution_resources::GasVector, transaction::fields::Tip};

impl<D: MadaraStorageRead> ExecutionContext<D> {
    pub fn execution_result_to_fee_estimate(
        &self,
        executions_result: &ExecutionResult,
        tip: Tip,
    ) -> Result<mp_rpc::v0_7_1::FeeEstimate, Error> {
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
                .context("Gas vector overflow")?;
        }

        let overall_fee = gas_vector.cost(gas_price_vector, tip);

        let unit = match executions_result.fee_type {
            FeeType::Eth => mp_rpc::v0_7_1::PriceUnit::Wei,
            FeeType::Strk => mp_rpc::v0_7_1::PriceUnit::Fri,
        };

        Ok(mp_rpc::v0_7_1::FeeEstimate {
            gas_consumed: gas_vector.l1_gas.0.into(),
            gas_price: gas_price_vector.l1_gas_price.get().0.into(),
            data_gas_consumed: gas_vector.l1_data_gas.0.into(),
            data_gas_price: gas_price_vector.l1_data_gas_price.get().0.into(),
            overall_fee: overall_fee.into(),
            unit,
        })
    }
}
