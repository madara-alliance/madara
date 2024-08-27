use anyhow::Context;
use dc_block_import::DeclaredClass;
use dp_class::ContractClass;
use dp_state_update::DeclaredClassItem;
use starknet_core::types::{BroadcastedDeclareTransaction, BroadcastedDeclareTransactionV3, BroadcastedTransaction};
use starknet_types_core::felt::Felt;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct InitiallyDeclaredSierraClass {
    pub class_hash: Felt,
    pub compiled_class_hash: Felt,
    pub definition: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct InitiallyDeclaredLegacyClass {
    pub class_hash: Felt,
    pub definition: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum InitiallyDeclaredClass {
    Sierra(InitiallyDeclaredSierraClass),
    Legacy(InitiallyDeclaredLegacyClass),
}

impl InitiallyDeclaredClass {
    pub fn new_sierra(class_hash: Felt, compiled_class_hash: Felt, definition: impl Into<Vec<u8>>) -> Self {
        Self::Sierra(InitiallyDeclaredSierraClass { class_hash, compiled_class_hash, definition: definition.into() })
    }
    pub fn new_legacy(class_hash: Felt, definition: impl Into<Vec<u8>>) -> Self {
        Self::Legacy(InitiallyDeclaredLegacyClass { class_hash, definition: definition.into() })
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

    /// Load the classes into `DeclaredClass`es.
    pub fn into_loaded_classes(self) -> anyhow::Result<Vec<DeclaredClass>> {
        use starknet_core::types::contract::{legacy::LegacyContractClass, SierraClass};

        self.0
            .into_iter()
            .enumerate()
            .map(|(i, (class_hash, class))| {
                let make_err_ctx =
                    |what: &'static str| move || format!("{what} class with hash {class_hash:#} (index {i})");
                match class {
                    InitiallyDeclaredClass::Sierra(c) => {
                        let compiled_class_hash = c.compiled_class_hash;
                        let class = serde_json::from_slice::<SierraClass>(&c.definition)
                            .with_context(make_err_ctx("Deserializing sierra"))?;
                        let class = class.flatten().with_context(make_err_ctx("Flattening sierra"))?;

                        Ok(DeclaredClass {
                            class_hash,
                            contract_class: ContractClass::Sierra(class.into()),
                            compiled_class_hash,
                        })
                    }
                    InitiallyDeclaredClass::Legacy(c) => {
                        let class = serde_json::from_slice::<LegacyContractClass>(&c.definition)
                            .with_context(make_err_ctx("Deserializing legacy"))?;
                        let class = class.compress().context("Compressing legacy")?;

                        Ok(DeclaredClass {
                            class_hash,
                            contract_class: ContractClass::Legacy(class.into()),
                            compiled_class_hash: Felt::ZERO, // no compilation
                        })
                    }
                }
            })
            .collect()
    }
}
