use std::path::PathBuf;

fn main() {
    if local_artifacts_available() {
        println!("cargo::warning=using existing local artifacts from ../build-artifacts");
        return;
    }

    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");
}

fn local_artifacts_available() -> bool {
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return false,
    };

    let Some(root_dir) = manifest_dir.parent() else {
        return false;
    };

    [
        "build-artifacts/starkgate_latest/erc20.sierra.json",
        "build-artifacts/starkgate_latest/token_bridge.sierra.json",
        "build-artifacts/starkgate_latest/eth_bridge_upgraded.json",
        "build-artifacts/starkgate_latest/eic_eth_bridge.json",
        "build-artifacts/starkgate_legacy/legacy_token_bridge.json",
        "build-artifacts/argent/ArgentAccount.sierra.json",
        "build-artifacts/braavos/BraavosAccount.sierra.json",
    ]
    .into_iter()
    .all(|path| root_dir.join(path).exists())
}
