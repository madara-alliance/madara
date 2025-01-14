use starknet_types_core::felt::Felt;

#[derive(Clone, Debug)]
pub enum ClassUpdate {
    Legacy(LegacyClassUpdate),
    Sierra(SierraClassUpdate),
}

impl ClassUpdate {
    pub fn class_hash(&self) -> Felt {
        match self {
            ClassUpdate::Legacy(legacy) => legacy.class_hash,
            ClassUpdate::Sierra(sierra) => sierra.class_hash,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LegacyClassUpdate {
    pub class_hash: Felt,
    pub contract_class: crate::CompressedLegacyContractClass,
}

#[derive(Clone, Debug)]
pub struct SierraClassUpdate {
    pub class_hash: Felt,
    pub contract_class: crate::FlattenedSierraClass,
    pub compiled_class_hash: Felt,
}
