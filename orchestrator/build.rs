/// The orchestrator crate does not `include_bytes!` anything from `build-artifacts`, but its
/// test-suite family (`make test-orchestrator` runs `--package "orchestrator*"`) reads the mock
/// Piltover contracts from `build-artifacts/orchestrator_tests` at runtime. Declaring them here
/// keeps the historical behavior of pre-fetching the artifacts image when something is missing,
/// while letting Docker-less hosts (e.g. macOS source builds) compile once the files exist on
/// disk, via `make fetch-artifacts-no-docker`.
const REQUIRED_ARTIFACTS: &[&str] = &[
    "build-artifacts/orchestrator_tests/mock_contracts.starknet_artifacts.json",
    "build-artifacts/orchestrator_tests/mock_contracts_Piltover.compiled_contract_class.json",
    "build-artifacts/orchestrator_tests/mock_contracts_Piltover.contract_class.json",
];

fn main() {
    if let Err(e) = build_version::get_or_compile_artifacts_for(1, REQUIRED_ARTIFACTS) {
        panic!("Failed to load artifacts: {e}");
    }
}
