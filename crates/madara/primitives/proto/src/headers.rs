use crate::{
    model::{self},
    FromModelError,
};
use mp_block::{
    header::{GasPrices, L1DataAvailabilityMode},
    BlockHeaderWithSignatures, ConsensusSignature, Header,
};

impl TryFrom<model::SignedBlockHeader> for BlockHeaderWithSignatures {
    type Error = FromModelError;
    fn try_from(value: model::SignedBlockHeader) -> Result<Self, Self::Error> {
        let transactions = value.transactions.unwrap_or_default();
        let events = value.events.unwrap_or_default();
        let state_diff_commitment = value.state_diff_commitment.unwrap_or_default();
        Ok(Self {
            header: Header {
                parent_block_hash: value.parent_hash.unwrap_or_default().into(),
                block_number: value.number,
                global_state_root: value.state_root.unwrap_or_default().into(),
                sequencer_address: value.sequencer_address.unwrap_or_default().into(),
                block_timestamp: mp_block::header::BlockTimestamp(value.time),
                transaction_count: transactions.n_leaves,
                transaction_commitment: transactions.root.unwrap_or_default().into(),
                event_count: events.n_leaves,
                event_commitment: events.root.unwrap_or_default().into(),
                state_diff_length: Some(state_diff_commitment.state_diff_length),
                state_diff_commitment: Some(state_diff_commitment.root.unwrap_or_default().into()),
                receipt_commitment: Some(value.receipts.unwrap_or_default().into()),
                protocol_version: value
                    .protocol_version
                    .parse()
                    .map_err(|_| FromModelError::invalid_field("SignedBlockHeader::protocol_version"))?,
                l1_gas_price: GasPrices {
                    eth_l1_gas_price: value.gas_price_wei.unwrap_or_default().into(),
                    strk_l1_gas_price: value.gas_price_fri.unwrap_or_default().into(),
                    eth_l1_data_gas_price: value.data_gas_price_wei.unwrap_or_default().into(),
                    strk_l1_data_gas_price: value.data_gas_price_fri.unwrap_or_default().into(),
                },
                l1_da_mode: model::L1DataAvailabilityMode::try_from(value.l1_data_availability_mode)
                    .map_err(|_| {
                        FromModelError::invalid_enum_variant("L1DataAvailabilityMode", value.l1_data_availability_mode)
                    })?
                    .into(),
            },
            block_hash: value.block_hash.unwrap_or_default().into(),
            consensus_signatures: value.signatures.into_iter().map(TryInto::try_into).collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<model::ConsensusSignature> for ConsensusSignature {
    type Error = FromModelError;
    fn try_from(value: model::ConsensusSignature) -> Result<Self, Self::Error> {
        Ok(Self { r: value.r.unwrap_or_default().into(), s: value.s.unwrap_or_default().into() })
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
            gas_price_fri: Some(val.header.l1_gas_price.strk_l1_gas_price.into()),
            gas_price_wei: Some(val.header.l1_gas_price.eth_l1_gas_price.into()),
            data_gas_price_fri: Some(val.header.l1_gas_price.strk_l1_data_gas_price.into()),
            data_gas_price_wei: Some(val.header.l1_gas_price.eth_l1_data_gas_price.into()),
            l1_data_availability_mode: model::L1DataAvailabilityMode::from(val.header.l1_da_mode).into(),
            signatures: val.consensus_signatures.into_iter().map(Into::into).collect(),
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
