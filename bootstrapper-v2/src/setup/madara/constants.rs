pub const BOOTSTRAP_ACCOUNT_SIERRA: &str =
    "contracts/madara/target/dev/madara_factory_contracts_Account.contract_class.json";
pub const BOOTSTRAP_ACCOUNT_CASM: &str =
    "contracts/madara/target/dev/madara_factory_contracts_Account.compiled_contract_class.json";
// Hex value of `BOOTSTRAP`
/// This is a special private key for the bootstrap account.
/// It can be used to make the first special declare txn without any validation.
/// Check this for [reference](https://github.com/starkware-libs/sequencer/blob/main/crates/starknet_api/src/executable_transaction.rs#L236C4-L239C6)
pub const BOOTSTRAP_PRIVATE_KEY: &str = "0x424f4f545354524150";
