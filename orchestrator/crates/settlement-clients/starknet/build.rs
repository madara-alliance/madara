/// Artifact files required by the tests in `src/tests/test.rs` (mock Piltover contracts, read at
/// runtime from `build-artifacts/orchestrator_tests`). They ship in the published artifacts
/// image, so a Docker-less host (e.g. a macOS source build) compiles after running
/// `make fetch-artifacts-no-docker`; the build script only reaches for Docker if they are
/// missing.
const REQUIRED_ARTIFACTS: &[&str] = &[
    "build-artifacts/orchestrator_tests/mock_contracts.starknet_artifacts.json",
    "build-artifacts/orchestrator_tests/mock_contracts_Piltover.compiled_contract_class.json",
    "build-artifacts/orchestrator_tests/mock_contracts_Piltover.contract_class.json",
];

fn main() {
    if let Err(e) = build_version::get_or_compile_artifacts_for(4, REQUIRED_ARTIFACTS) {
        panic!("Failed to load artifacts: {e}");
    }
}
