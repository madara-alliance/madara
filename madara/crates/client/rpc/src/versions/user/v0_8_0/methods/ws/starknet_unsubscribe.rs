pub async fn starknet_unsubscribe(starknet: &crate::Starknet, subscription_id: u64) -> crate::StarknetRpcResult<bool> {
    if starknet.ws_handles.subscription_close(subscription_id).await {
        Ok(true)
    } else {
        Err(crate::StarknetRpcApiError::InvalidSubscriptionId)
    }
}

#[cfg(test)]
mod test {
    #[rstest::fixture]
    fn logs() {
        let debug = tracing_subscriber::filter::LevelFilter::DEBUG;
        let env = tracing_subscriber::EnvFilter::builder().with_default_directive(debug.into()).from_env_lossy();
        let _ = tracing_subscriber::fmt().with_test_writer().with_env_filter(env).with_line_number(true).try_init();
    }

    #[rstest::fixture]
    fn starknet() -> crate::Starknet {
        let chain_config = std::sync::Arc::new(mp_chain_config::ChainConfig::madara_test());
        let backend = mc_db::MadaraBackend::open_for_testing(chain_config);
        let validation = mc_submit_tx::TransactionValidatorConfig { disable_validation: true, disable_fee: true };
        let mempool = std::sync::Arc::new(mc_mempool::Mempool::new(
            std::sync::Arc::clone(&backend),
            mc_mempool::MempoolConfig::for_testing(),
        ));
        let mempool_validator = std::sync::Arc::new(mc_submit_tx::TransactionValidator::new(
            mempool,
            std::sync::Arc::clone(&backend),
            validation,
        ));
        let context = mp_utils::service::ServiceContext::new_for_testing();

        crate::Starknet::new(backend, mempool_validator, Default::default(), None, context)
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn starknet_unsubscribe_err(_logs: (), starknet: crate::Starknet) {
        assert_eq!(
            super::starknet_unsubscribe(&starknet, 0).await,
            Err(crate::StarknetRpcApiError::InvalidSubscriptionId)
        )
    }
}
