use crate::prelude::*;
use bigdecimal::{ToPrimitive, Zero};
use mp_block::{header::GasPrices, L1GasQuote, U256};

#[derive(Debug)]
pub(super) struct L1GasQuoteCell(tokio::sync::watch::Sender<Option<L1GasQuote>>);

impl Default for L1GasQuoteCell {
    fn default() -> Self {
        Self(tokio::sync::watch::Sender::new(None))
    }
}

impl<D: MadaraStorageRead> MadaraBackend<D> {
    pub fn set_last_l1_gas_quote(&self, quote: L1GasQuote) {
        self.watch_gas_quote.0.send_replace(Some(quote));
    }
    pub fn get_last_l1_gas_quote(&self) -> Option<L1GasQuote> {
        self.watch_gas_quote.0.borrow().clone()
    }

    #[cfg(feature = "testing")]
    pub fn set_l1_gas_quote_for_testing(&self) {
        use mp_convert::FixedPoint;

        let l1_gas_quote = L1GasQuote { l1_gas_price: 128, l1_data_gas_price: 128, strk_per_eth: FixedPoint::one() };
        self.set_last_l1_gas_quote(l1_gas_quote);
    }

    pub fn calculate_gas_prices(
        &self,
        previous_strk_l2_gas_price: u128,
        previous_l2_gas_used: u128,
    ) -> anyhow::Result<GasPrices> {
        let l1_gas_quote = self.get_last_l1_gas_quote().ok_or_else(|| {
            anyhow::anyhow!(
                "No L1 gas quote available. Ensure that the L1 gas quote is set before calculating gas prices."
            )
        })?;
        let eth_l1_gas_price = l1_gas_quote.l1_gas_price;
        let eth_l1_data_gas_price = l1_gas_quote.l1_data_gas_price;
        let strk_per_eth = {
            let strk_per_eth = l1_gas_quote.strk_per_eth;
            bigdecimal::BigDecimal::new(strk_per_eth.value().into(), strk_per_eth.decimals().into())
        };
        let strk_l1_gas_price = (&bigdecimal::BigDecimal::from(eth_l1_gas_price) * &strk_per_eth)
            .to_u128()
            .context("Failed to convert STRK L1 gas price to u128")?;
        let strk_l1_data_gas_price = (&bigdecimal::BigDecimal::from(eth_l1_data_gas_price) * &strk_per_eth)
            .to_u128()
            .context("Failed to convert STRK L1 data gas price to u128")?;

        let l2_gas_target = self.chain_config().l2_gas_target;
        let max_change_denominator = self.chain_config().l2_gas_price_max_change_denominator;
        let strk_l2_gas_price = calculate_gas_price(
            previous_strk_l2_gas_price,
            previous_l2_gas_used,
            l2_gas_target,
            max_change_denominator,
        )?
        .max(self.chain_config().min_l2_gas_price);
        if strk_per_eth.is_zero() {
            anyhow::bail!("STRK per ETH is zero, cannot calculate gas prices")
        }
        let eth_l2_gas_price = (&bigdecimal::BigDecimal::from(strk_l2_gas_price) / &strk_per_eth)
            .to_u128()
            .context("Failed to convert ETH L2 gas price to u128")?;

        Ok(GasPrices {
            eth_l1_gas_price,
            strk_l1_gas_price,
            eth_l1_data_gas_price,
            strk_l1_data_gas_price,
            eth_l2_gas_price,
            strk_l2_gas_price,
        })
    }
}

fn calculate_gas_price(
    previous_gas_price: u128,
    previous_gas_used: u128,
    target_gas_used: u128,
    max_change_denominator: u128,
) -> Result<u128> {
    ensure!(max_change_denominator > 0, "max_change_denominator must be greater than 0");
    ensure!(target_gas_used > 0, "target_gas_used must be greater than 0");
    let delta = previous_gas_used.abs_diff(target_gas_used);
    let price_change = ((U256::from(previous_gas_price)).saturating_mul(U256::from(delta)))
        .checked_div(U256::from((target_gas_used).saturating_mul(max_change_denominator)))
        .context("Failed to calculate price change")?
        .try_into()
        .map_err(|err: &str| anyhow::anyhow!(err))
        .context("Failed to convert price change to u128")?;

    if previous_gas_used > target_gas_used {
        Ok(previous_gas_price.saturating_add(price_change))
    } else {
        Ok(previous_gas_price.saturating_sub(price_change))
    }
}
