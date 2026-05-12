use super::ensure_rpc_unsafe_enabled;
use crate::versions::admin::v0_1_0::MadaraReplayWriteRpcApiV0_1_0Server;
use crate::{Starknet, StarknetRpcApiError};
use jsonrpsee::core::{async_trait, RpcResult};
use mp_block::header::CustomHeader;

#[async_trait]
impl MadaraReplayWriteRpcApiV0_1_0Server for Starknet {
    async fn set_block_header(&self, custom_block_headers: CustomHeader) -> RpcResult<()> {
        ensure_rpc_unsafe_enabled(self)?;
        self.backend.set_custom_header(custom_block_headers).map_err(StarknetRpcApiError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        test_utils::TestTransactionProvider, versions::admin::v0_1_0::MadaraReplayWriteRpcApiV0_1_0Server, Starknet,
    };
    use mc_db::{test_utils::add_test_block, MadaraBackend};
    use mp_block::header::{CustomHeader, GasPrices};
    use mp_chain_config::ChainConfig;
    use mp_convert::Felt;
    use mp_utils::service::ServiceContext;
    use std::sync::Arc;

    fn make_starknet(backend: Arc<MadaraBackend>, ctx: ServiceContext) -> Starknet {
        let mut rpc = Starknet::new(backend, Arc::new(TestTransactionProvider), Default::default(), None, ctx);
        rpc.set_rpc_unsafe_enabled(true);
        rpc
    }

    #[tokio::test]
    async fn set_block_header_updates_fake_preconfirmed_view() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        add_test_block(&backend, 0, vec![]);

        let rpc = make_starknet(backend.clone(), ServiceContext::default());
        let custom_header = CustomHeader {
            block_n: 1,
            timestamp: 1_234_567_890,
            gas_prices: GasPrices {
                eth_l1_gas_price: 11,
                strk_l1_gas_price: 12,
                eth_l1_data_gas_price: 21,
                strk_l1_data_gas_price: 22,
                eth_l2_gas_price: 31,
                strk_l2_gas_price: 32,
            },
            expected_block_hash: Felt::from(0x1234_u64),
        };

        rpc.set_block_header(custom_header.clone()).await.expect("set block header should succeed");

        let preconfirmed =
            backend.block_view_on_preconfirmed_or_fake().expect("fake preconfirmed block should always be available");

        assert_eq!(preconfirmed.block_number(), custom_header.block_n);
        assert_eq!(preconfirmed.header().block_timestamp.0, custom_header.timestamp);
        assert_eq!(preconfirmed.header().gas_prices, custom_header.gas_prices);
    }
}
