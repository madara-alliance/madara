use anyhow::Context;
use mp_class::{
    ClassInfo, ClassInfoWithHash, CompressedLegacyContractClass, FlattenedSierraClass, LegacyClassInfo,
    LegacyContractClass, SierraClassInfo,
};
use mp_state_update::DeclaredClassItem;
use starknet_core::types::contract::SierraClass;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct InitiallyDeclaredSierraClass {
    pub contract_class: FlattenedSierraClass,
    pub class_hash: Felt,
    pub compiled_class_hash: Felt,
}

#[derive(Clone, Debug)]
pub struct InitiallyDeclaredLegacyClass {
    pub contract_class: CompressedLegacyContractClass,
    pub class_hash: Felt,
}

#[derive(Clone, Debug)]
pub enum InitiallyDeclaredClass {
    Sierra(InitiallyDeclaredSierraClass),
    Legacy(InitiallyDeclaredLegacyClass),
}

impl InitiallyDeclaredClass {
    pub fn new_sierra(definition: impl Into<Vec<u8>>) -> anyhow::Result<Self> {
        let class = serde_json::from_slice::<SierraClass>(&definition.into())
            .with_context(|| "Deserializing sierra class".to_string())?;
        let contract_class: FlattenedSierraClass =
            class.flatten().with_context(|| "Flattening sierra class".to_string())?.into();
        let class_hash = contract_class.compute_class_hash().context("Computing sierra class hash")?;
        let (compiled_class_hash, _) = contract_class.compile_to_casm().context("Compiling sierra class")?;

        Ok(Self::Sierra(InitiallyDeclaredSierraClass { contract_class, class_hash, compiled_class_hash }))
    }
    pub fn new_legacy(definition: impl Into<Vec<u8>>) -> anyhow::Result<Self> {
        let contract_class = serde_json::from_slice::<LegacyContractClass>(&definition.into())
            .with_context(|| "Deserializing legacy class".to_string())?;
        let class_hash = contract_class.class_hash().context("Computing legacy class hash")?;
        let contract_class = contract_class.compress().context("Compressing legacy")?.into();

        Ok(Self::Legacy(InitiallyDeclaredLegacyClass { contract_class, class_hash }))
    }

    pub fn class_hash(&self) -> Felt {
        match self {
            InitiallyDeclaredClass::Sierra(c) => c.class_hash,
            InitiallyDeclaredClass::Legacy(c) => c.class_hash,
        }
    }
    pub fn compiled_class_hash(&self) -> Felt {
        match self {
            InitiallyDeclaredClass::Sierra(c) => c.compiled_class_hash,
            InitiallyDeclaredClass::Legacy(_) => Felt::ZERO,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct InitiallyDeclaredClasses(HashMap<Felt, InitiallyDeclaredClass>);

impl InitiallyDeclaredClasses {
    pub fn with(mut self, class: InitiallyDeclaredClass) -> Self {
        self.insert(class);
        self
    }

    pub fn insert(&mut self, class: InitiallyDeclaredClass) {
        self.0.insert(class.class_hash(), class);
    }

    pub fn as_state_diff(&self) -> Vec<DeclaredClassItem> {
        self.0
            .iter()
            .filter(|(_, class)| matches!(class, InitiallyDeclaredClass::Sierra(_)))
            .map(|(&class_hash, class)| DeclaredClassItem {
                class_hash,
                compiled_class_hash: class.compiled_class_hash(),
            })
            .collect()
    }

    /// This is for the LegacyDeclaredClasses field in State diff.
    pub fn as_legacy_state_diff(&self) -> Vec<Felt> {
        self.0
            .iter()
            .filter(|(_, class)| matches!(class, InitiallyDeclaredClass::Legacy(_)))
            .map(|(&class_hash, _class)| class_hash)
            .collect()
    }

    pub fn into_class_infos(self) -> Vec<ClassInfoWithHash> {
        self.0
            .into_values()
            .map(|class| match class {
                InitiallyDeclaredClass::Sierra(c) => ClassInfoWithHash {
                    class_hash: c.class_hash,
                    class_info: ClassInfo::Sierra(SierraClassInfo {
                        contract_class: c.contract_class.into(),
                        compiled_class_hash: Some(c.compiled_class_hash),
                        compiled_class_hash_v2: None,
                    }),
                },
                InitiallyDeclaredClass::Legacy(c) => ClassInfoWithHash {
                    class_hash: c.class_hash,
                    class_info: ClassInfo::Legacy(LegacyClassInfo { contract_class: c.contract_class.into() }),
                },
            })
            .collect()
    }
}
