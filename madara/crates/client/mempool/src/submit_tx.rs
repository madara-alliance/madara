impl SubmitValidatedTransaction for GatewayProvider {
    async fn submit_validated_transaction(
        &self,
        tx: ValidatedMempoolTx,
    ) -> Result<(), SubmitTransactionError> {
        self.add_validated_transaction(tx).await.map_err(map_gateway_error)
    }
}