//! This crate is used to build the cairo test contracts for tests. Contracts that are used in the normal
//! function of madara (ie. not tests) need to be put in the `cairo-artifacts` folder at the root of the project.
//! This is so we only build these contracts with scarb when compiling tests.

pub const TEST_CONTRACT_SIERRA: &[u8] =
    include_bytes!("../../../../cairo/target/dev/madara_contracts_TestContract.contract_class.json");
