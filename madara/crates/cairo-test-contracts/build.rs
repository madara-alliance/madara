/// Artifact files required by `src/lib.rs` (`include_bytes!`). These are tracked in git, so a
/// fresh source checkout builds without Docker; the build script only reaches for Docker if they
/// are missing.
const REQUIRED_ARTIFACTS: &[&str] = &[
    "build-artifacts/js_tests/madara_contracts_TestContract.contract_class.json",
    "build-artifacts/js_tests/madara_contracts_StateUpdateContract.contract_class.json",
    "build-artifacts/js_tests/madara_contracts_MessagingContract.contract_class.json",
];

fn main() {
    if let Err(e) = build_version::get_or_compile_artifacts_for(3, REQUIRED_ARTIFACTS) {
        panic!("Failed to load artifacts: {e}");
    }
}
