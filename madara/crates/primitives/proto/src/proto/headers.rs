use crate::{
    model::{self},
    model_field, model_field_variant, CollectInto, FromModelError,
};
use m_proc_macros::model_describe;
use mp_block::{
    header::{GasPrices, L1DataAvailabilityMode},
    BlockHeaderWithSignatures, ConsensusSignature, Header,
};

impl TryFrom<model::SignedBlockHeader> for BlockHeaderWithSignatures {
    type Error = FromModelError;

    #[model_describe(model::SignedBlockHeader)]
    fn try_from(value: model::SignedBlockHeader) -> Result<Self, Self::Error> {
        let transactions = model_field!(value => transactions);
        let events = model_field!(value => events);
        let state_diff_commitment = model_field!(value => state_diff_commitment);
        Ok(Self {
            header: Header {
                parent_block_hash: model_field!(value => parent_hash).into(),
                block_number: value.number,
                global_state_root: model_field!(value => state_root).into(),
                sequencer_address: model_field!(value => sequencer_address).into(),
                block_timestamp: mp_block::header::BlockTimestamp(value.time),
                transaction_count: transactions.n_leaves,
                transaction_commitment: model_field!(transactions => root).into(),
                event_count: events.n_leaves,
                event_commitment: model_field!(events => root).into(),
                state_diff_length: Some(state_diff_commitment.state_diff_length),
                state_diff_commitment: Some(model_field!(state_diff_commitment => root).into()),
                receipt_commitment: Some(model_field!(value => receipts).into()),
                protocol_version: value
                    .protocol_version
                    .parse()
                    .map_err(|_| FromModelError::invalid_field("protocol_version"))?,
                l1_gas_price: GasPrices {
                    eth_l1_gas_price: model_field!(value => l1_gas_price_wei).into(),
                    strk_l1_gas_price: model_field!(value => l1_gas_price_fri).into(),
                    eth_l1_data_gas_price: model_field!(value => l1_data_gas_price_wei).into(),
                    strk_l1_data_gas_price: model_field!(value => l1_data_gas_price_fri).into(),
                },
                l1_da_mode: model_field_variant!(model::L1DataAvailabilityMode => value.l1_data_availability_mode)
                    .into(),
            },
            block_hash: model_field!(value => block_hash).into(),
            consensus_signatures: value.signatures.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<model::ConsensusSignature> for ConsensusSignature {
    type Error = FromModelError;

    #[model_describe(model::ConsensusSignature)]
    fn try_from(value: model::ConsensusSignature) -> Result<Self, Self::Error> {
        Ok(Self { r: model_field!(value => r).into(), s: model_field!(value => s).into() })
    }
}

impl From<BlockHeaderWithSignatures> for model::SignedBlockHeader {
    fn from(val: BlockHeaderWithSignatures) -> Self {
        model::SignedBlockHeader {
            block_hash: Some(val.block_hash.into()),
            parent_hash: Some(val.header.parent_block_hash.into()),
            number: val.header.block_number,
            time: val.header.block_timestamp.0,
            sequencer_address: Some(val.header.sequencer_address.into()),
            state_root: Some(val.header.global_state_root.into()),
            state_diff_commitment: val.header.state_diff_commitment.zip(val.header.state_diff_length).map(
                |(commitment, state_diff_length)| model::StateDiffCommitment {
                    state_diff_length,
                    root: Some(commitment.into()),
                },
            ),
            transactions: Some(model::Patricia {
                n_leaves: val.header.transaction_count,
                root: Some(val.header.transaction_commitment.into()),
            }),
            events: Some(model::Patricia {
                n_leaves: val.header.event_count,
                root: Some(val.header.event_commitment.into()),
            }),
            receipts: val.header.receipt_commitment.map(Into::into),
            protocol_version: val.header.protocol_version.to_string(),
            l1_gas_price_fri: Some(val.header.l1_gas_price.strk_l1_gas_price.into()),
            l1_gas_price_wei: Some(val.header.l1_gas_price.eth_l1_gas_price.into()),
            l1_data_gas_price_fri: Some(val.header.l1_gas_price.strk_l1_data_gas_price.into()),
            l1_data_gas_price_wei: Some(val.header.l1_gas_price.eth_l1_data_gas_price.into()),
            l1_data_availability_mode: model::L1DataAvailabilityMode::from(val.header.l1_da_mode).into(),
            l2_gas_price_fri: None, // TODO: update blockifier
            l2_gas_price_wei: None,
            signatures: val.consensus_signatures.collect_into(),
        }
    }
}

impl From<ConsensusSignature> for model::ConsensusSignature {
    fn from(value: ConsensusSignature) -> Self {
        Self { r: Some(value.r.into()), s: Some(value.s.into()) }
    }
}

impl From<L1DataAvailabilityMode> for model::L1DataAvailabilityMode {
    fn from(value: L1DataAvailabilityMode) -> Self {
        match value {
            L1DataAvailabilityMode::Calldata => Self::Calldata,
            L1DataAvailabilityMode::Blob => Self::Blob,
        }
    }
}

impl From<model::L1DataAvailabilityMode> for L1DataAvailabilityMode {
    fn from(value: model::L1DataAvailabilityMode) -> Self {
        match value {
            model::L1DataAvailabilityMode::Calldata => Self::Calldata,
            model::L1DataAvailabilityMode::Blob => Self::Blob,
        }
    }
}
