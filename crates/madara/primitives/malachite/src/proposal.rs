use mp_block::MadaraBlock;

use crate::{
    context::MadaraContext,
    types::{Address, Height},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProposalPartStub;

impl malachite_core_types::ProposalPart<MadaraContext> for ProposalPartStub {
    fn is_first(&self) -> bool {
        unimplemented!()
    }

    fn is_last(&self) -> bool {
        unimplemented!()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProposalStub;

impl malachite_core_types::Proposal<MadaraContext> for ProposalStub {
    fn height(&self) -> Height {
        todo!()
    }

    fn round(&self) -> malachite_core_types::Round {
        todo!()
    }

    fn value(&self) -> &MadaraBlock {
        todo!()
    }

    fn take_value(self) -> MadaraBlock {
        todo!()
    }

    fn pol_round(&self) -> malachite_core_types::Round {
        todo!()
    }

    fn validator_address(&self) -> &Address {
        todo!()
    }
}
