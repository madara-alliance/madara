//! Parser tests for the prover-selection CLI surface.
//!
//! Locks down the guarantees that used to be given by the old `ArgGroup` on
//! `--sharp` / `--atlantic` / `--mock` booleans. The new surface relies on:
//! 1. A required `ProverKind` enum field at the `RunCmd` level.
//! 2. `required_if_eq("prover", "<kind>")` on the per-prover CLI structs.
//!
//! We drive `Cli::try_parse_from` end-to-end with a minimal baseline of args
//! that satisfies every non-prover `ArgGroup`, then vary only the prover
//! portion per assertion.

use clap::Parser;

use crate::cli::Cli;

/// Strip all `MADARA_ORCHESTRATOR_*` env vars so `try_parse_from` only sees
/// what we pass on the CLI. Without this, a developer's shell or CI env
/// could satisfy a required arg through the env fallback and mask a bug.
fn strip_madara_envs() {
    let keys: Vec<String> =
        std::env::vars().map(|(k, _)| k).filter(|k| k.starts_with("MADARA_ORCHESTRATOR_")).collect();
    for k in keys {
        std::env::remove_var(&k);
    }
}

/// Args that satisfy every non-prover required group on `RunCmd`:
/// provider / storage / queue / alert / settlement_layer / da_layer, plus
/// the standalone `--madara-rpc-url`.
fn baseline() -> Vec<&'static str> {
    vec![
        "orchestrator",
        "run",
        "--aws",
        "--aws-s3",
        "--aws-sqs",
        "--aws-sns",
        "--settle-on-ethereum",
        "--da-on-ethereum",
        "--madara-rpc-url",
        "http://localhost:9944",
        "--ethereum-da-rpc-url",
        "http://localhost:8545",
        "--rpc-for-snos",
        "http://localhost:9545",
        "--max-batch-time-seconds",
        "60",
        "--max-batch-size",
        "10",
        "--queue-identifier",
        "test",
    ]
}

/// Required-when-sharp sub-args that `required_if_eq("prover", "sharp")` enforces.
/// Cert material is file-path only on the SHARP path — dummy paths are fine
/// here because `try_parse_from` doesn't open the files (that happens later
/// in `validate_sharp`). `--sharp-settlement-layer` has a default so it's
/// not required.
fn sharp_required_args() -> Vec<&'static str> {
    vec![
        "--sharp-customer-id",
        "test-customer",
        "--sharp-url",
        "http://sharp.example/",
        "--sharp-user-crt-file",
        "/tmp/dummy-crt.pem",
        "--sharp-user-key-file",
        "/tmp/dummy-key.pem",
        "--sharp-rpc-node-url",
        "http://rpc.example/",
        "--sharp-server-crt-file",
        "/tmp/dummy-server-crt.pem",
        "--gps-verifier-contract-address",
        "0x0000000000000000000000000000000000000000",
    ]
}

/// Required-when-atlantic sub-args.
fn atlantic_required_args() -> Vec<&'static str> {
    vec![
        "--atlantic-service-url",
        "http://atlantic.example/",
        "--atlantic-rpc-node-url",
        "http://rpc.example/",
        "--atlantic-mock-fact-hash",
        "true",
        "--atlantic-prover-type",
        "starkware_sharp",
        "--atlantic-settlement-layer",
        "ethereum",
        "--atlantic-verifier-contract-address",
        "0x0000000000000000000000000000000000000000",
        "--atlantic-network",
        "TESTNET",
        "--atlantic-verifier-cairo-vm",
        "rust",
        "--atlantic-verifier-result",
        "proof-generation",
    ]
}

/// Single test; env stripping + assertions live in one function to keep the
/// process-global env manipulation serialized regardless of how tests run.
#[test]
fn prover_selection_parser() {
    strip_madara_envs();

    // 1. Missing --prover: clap should refuse, and the message should
    // mention the prover arg so operators can actually diagnose it.
    let err = Cli::try_parse_from(baseline()).expect_err("expected clap to reject missing --prover");
    let msg = err.to_string().to_lowercase();
    assert!(msg.contains("prover"), "missing-prover error must mention `prover`; got: {}", err);

    // 2. Invalid enum value: clap should refuse.
    let mut args = baseline();
    args.extend(["--prover", "foo"]);
    Cli::try_parse_from(args).expect_err("expected clap to reject --prover foo");

    // 3. --prover sharp without required sub-args: should fail.
    let mut args = baseline();
    args.extend(["--prover", "sharp"]);
    Cli::try_parse_from(args).expect_err("expected clap to reject --prover sharp without sub-args");

    // 4. --prover sharp with all required sub-args: should parse cleanly.
    let mut args = baseline();
    args.extend(["--prover", "sharp"]);
    args.extend(sharp_required_args());
    Cli::try_parse_from(args).expect("sharp with required sub-args should parse");

    // 5. --prover atlantic without required sub-args: should fail.
    let mut args = baseline();
    args.extend(["--prover", "atlantic"]);
    Cli::try_parse_from(args).expect_err("expected clap to reject --prover atlantic without sub-args");

    // 6. --prover atlantic with all required sub-args: should parse cleanly.
    let mut args = baseline();
    args.extend(["--prover", "atlantic"]);
    args.extend(atlantic_required_args());
    Cli::try_parse_from(args).expect("atlantic with required sub-args should parse");

    // 7. --prover mock has no required sub-args.
    let mut args = baseline();
    args.extend(["--prover", "mock"]);
    Cli::try_parse_from(args).expect("mock should parse without sub-args");

    // 8. `ignore_case = true`: uppercase values must be accepted so operators
    // can set `MADARA_ORCHESTRATOR_PROVER=SHARP` by convention without a
    // silent fallback.
    let mut args = baseline();
    args.extend(["--prover", "MOCK"]);
    Cli::try_parse_from(args).expect("uppercase --prover MOCK should parse (ignore_case=true)");
}
