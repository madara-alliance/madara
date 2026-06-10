/// Artifact files required by `src/lib.rs` (`include_bytes!`). These are tracked in git, so a
/// fresh source checkout builds without Docker; the build script only reaches for Docker if they
/// are missing.
const REQUIRED_ARTIFACTS: &[&str] = &[
    "build-artifacts/cairo_artifacts/madara_contracts_UDC.json",
    "build-artifacts/cairo_artifacts/openzeppelin_ERC20Upgradeable.contract_class.json",
    "build-artifacts/cairo_artifacts/openzeppelin_AccountUpgradeable.contract_class.json",
];

fn main() {
    if let Err(e) = build_version::get_or_compile_artifacts_for(4, REQUIRED_ARTIFACTS) {
        panic!("Failed to load artifacts: {e}");
    }
}
