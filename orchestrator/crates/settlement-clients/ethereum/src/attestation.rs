use super::*;
use alloy::{consensus::TxEip1559, primitives::TxKind, sol, sol_types::SolCall};
use kzg_attestation_protocol::{Certificate, Policy};
use std::result::Result::Ok;

sol! {
    #[sol(rpc)]
    interface AttestedCore {
        function programHash() external view returns (uint256);
        function aggregatorProgramHash() external view returns (uint256);
        function configHash() external view returns (uint256);
        function blobAttestationConfig() external view returns (uint256 epoch, uint256 threshold);
        function isBlobAttestor(address member) external view returns (bool);
        function updateStateWithBlobAttestations(uint256[] programOutput, uint256 committeeEpoch, bytes[] signatures) external;
    }
}

impl EthereumSettlementClient {
    pub(super) async fn check_attestation_policy(&self, policy: &Policy) -> Result<()> {
        policy.validate()?;
        if policy.core_address != self.core_contract_client.contract_address()
            || policy.chain_id != self.provider.get_chain_id().await?
        {
            bail!("Attestation policy chain or core differs from settlement client");
        }
        let core = AttestedCore::new(policy.core_address, self.provider.clone());
        // Pin all calls to one block so committee changes cannot produce a mixed policy view.
        let block = self.provider.get_block_number().await?;
        let block_id = alloy::eips::BlockId::number(block);
        let committee = core.blobAttestationConfig().block(block_id).call().await?;
        if committee.epoch != U256::from(policy.committee_epoch)
            || committee.threshold != U256::from(policy.threshold)
            || core.programHash().block(block_id).call().await? != U256::from_be_bytes(policy.os_program_hash.0)
            || core.aggregatorProgramHash().block(block_id).call().await?
                != U256::from_be_bytes(policy.aggregator_program_hash.0)
            || core.configHash().block(block_id).call().await? != U256::from_be_bytes(policy.config_hash.0)
        {
            bail!("Attestation policy does not match the on-chain core");
        }
        for member in &policy.members {
            if !core.isBlobAttestor(*member).block(block_id).call().await? {
                bail!("Configured signer is not an active on-chain attestor");
            }
        }
        Ok(())
    }

    pub(super) async fn create_attested_transaction(
        &self,
        program_output: &[[u8; 32]],
        certificate: &Certificate,
        nonce: u64,
        replacement_fee_floor: Option<StateUpdateFeeCaps>,
    ) -> Result<PreparedStateUpdateTransaction> {
        let chain_id = self.provider.get_chain_id().await?;
        let base_fee = self
            .provider
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .ok_or_else(|| eyre!("Latest Ethereum block not found"))?
            .header()
            .base_fee_per_gas()
            .ok_or_else(|| eyre!("Missing EIP-1559 base fee"))?;
        let fees = StateUpdateFeeCaps {
            max_fee_per_gas: Self::initial_max_fee_per_gas(base_fee.into()),
            max_priority_fee_per_gas: INITIAL_MAX_PRIORITY_FEE_PER_GAS,
            max_fee_per_blob_gas: 0,
        };
        let fees = replacement_fee_floor.map_or(fees, |floor| Self::max_fee_caps(fees, floor));
        Self::ensure_l2_state_update_fee_within_cap(fees, 0, self.l2_state_update_max_fee_wei)?;
        let call = AttestedCore::updateStateWithBlobAttestationsCall {
            programOutput: program_output.iter().map(|word| U256::from_be_bytes(*word)).collect(),
            committeeEpoch: U256::from(certificate.committee_epoch),
            signatures: certificate.signatures.clone(),
        };
        let mut tx = TxEip1559 {
            chain_id,
            nonce,
            gas_limit: GAS_LIMIT_STATE_UPDATE,
            max_fee_per_gas: fees.max_fee_per_gas,
            max_priority_fee_per_gas: fees.max_priority_fee_per_gas,
            to: TxKind::Call(self.core_contract_client.contract_address()),
            value: U256::ZERO,
            access_list: AccessList::default(),
            input: call.abi_encode().into(),
        };
        let signature = self.wallet.default_signer().sign_transaction(&mut tx).await?;
        Ok(PreparedStateUpdateTransaction {
            tx_envelope: StateUpdateEnvelope::Attested(tx.into_signed(signature)),
            fee_caps: fees,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{consensus::Transaction, node_bindings::Anvil};

    #[tokio::test]
    async fn attestation_transaction_is_type_two_and_obeys_nonce_and_fee_cap() {
        let anvil = Anvil::new().try_spawn().expect("start local Anvil");
        let mut args = EthereumSettlementValidatedArgs {
            ethereum_rpc_url: anvil.endpoint_url(),
            ethereum_private_key: hex::encode(anvil.keys()[0].to_bytes()),
            l1_core_contract_address: Address::repeat_byte(17),
            starknet_operator_address: anvil.addresses()[0],
            ethereum_finality_retry_wait_in_secs: 1,
            ethereum_tx_confirmation_timeout_secs: 10,
            ethereum_max_fee_bumps: 2,
            ethereum_l2_state_update_max_fee_wei: DEFAULT_L2_STATE_UPDATE_MAX_FEE_WEI,
            disable_peerdas: false,
        };
        let client = EthereumSettlementClient::new_with_args(&args);
        let certificate =
            Certificate { digest: B256::ZERO, committee_epoch: 7, signatures: vec![Bytes::from(vec![1; 65])] };
        let output = vec![[0u8; 32]; 18];
        let prepared = client.create_attested_transaction(&output, &certificate, 12, None).await.unwrap();
        assert_eq!(prepared.fee_caps.max_fee_per_blob_gas, 0);
        let StateUpdateEnvelope::Attested(transaction) = prepared.tx_envelope else { panic!("blob envelope") };
        assert_eq!(transaction.encoded_2718()[0], 2);
        assert_eq!(transaction.tx().nonce(), 12);
        assert_eq!(transaction.tx().chain_id(), Some(anvil.chain_id()));
        let call = AttestedCore::updateStateWithBlobAttestationsCall::abi_decode(transaction.tx().input()).unwrap();
        assert_eq!(call.committeeEpoch, U256::from(7));
        assert_eq!(call.signatures, certificate.signatures);
        args.ethereum_l2_state_update_max_fee_wei = 1;
        let client = EthereumSettlementClient::new_with_args(&args);
        assert!(client.create_attested_transaction(&output, &certificate, 12, None).await.is_err());
    }
}
