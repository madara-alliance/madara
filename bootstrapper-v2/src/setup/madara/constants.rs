pub const BOOTSTRAP_ACCOUNT_SIERRA: &str =
    "contracts/madara/target/dev/madara_factory_contracts_Account.contract_class.json";
pub const BOOTSTRAP_ACCOUNT_CASM: &str =
    "contracts/madara/target/dev/madara_factory_contracts_Account.compiled_contract_class.json";
// Hex value of `BOOTSTRAP`
/// This is a special private key for the bootstrap account.
/// It can be used to make the first special declare txn without any validation.
/// Check this for [reference](https://github.com/starkware-libs/sequencer/blob/main/crates/starknet_api/src/executable_transaction.rs#L236C4-L239C6)
pub const BOOTSTRAP_PRIVATE_KEY: &str = "0x424f4f545354524150";

// Token Bridge artifacts
pub const TOKEN_BRIDGE_SIERRA: &str = "../build-artifacts/starkgate_latest/cairo/token_bridge.sierra.json";
pub const TOKEN_BRIDGE_CASM: &str = "../build-artifacts/starkgate_latest/cairo/token_bridge.casm.json";

// ERC20 artifacts
pub const ERC20_SIERRA: &str = "../build-artifacts/starkgate_latest/cairo/ERC20_070.sierra.json";
pub const ERC20_CASM: &str = "../build-artifacts/starkgate_latest/cairo/ERC20_070.casm.json";

// EIC artifacts
pub const EIC_SIERRA: &str = "./contracts/madara/target/dev/madara_factory_contracts_EIC.contract_class.json";
pub const EIC_CASM: &str = "./contracts/madara/target/dev/madara_factory_contracts_EIC.compiled_contract_class.json";

// Universal Deployer artifacts
pub const UNIVERSAL_DEPLOYER_SIERRA: &str =
    "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.contract_class.json";
pub const UNIVERSAL_DEPLOYER_CASM: &str =
    "./contracts/madara/target/dev/madara_factory_contracts_UniversalDeployer.compiled_contract_class.json";

// Madara Factory artifacts
pub const MADARA_FACTORY_SIERRA: &str =
    "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.contract_class.json";
pub const MADARA_FACTORY_CASM: &str =
    "./contracts/madara/target/dev/madara_factory_contracts_MadaraFactory.compiled_contract_class.json";
