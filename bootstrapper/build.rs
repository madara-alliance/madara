use std::path::{Path, PathBuf};

const USE_LOCAL_ARTIFACTS_ENV: &str = "MADARA_BOOTSTRAPPER_USE_LOCAL_ARTIFACTS";

fn main() {
    println!("cargo::rerun-if-env-changed={USE_LOCAL_ARTIFACTS_ENV}");

    if use_local_artifacts() && local_artifacts_available() {
        println!("cargo::warning=using existing local artifacts from ../build-artifacts");
        return;
    }

    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");
}

fn local_artifacts_available() -> bool {
    let Some(root_dir) = root_dir() else {
        return false;
    };

    local_artifacts_available_at(&root_dir)
}

fn use_local_artifacts() -> bool {
    matches!(std::env::var(USE_LOCAL_ARTIFACTS_ENV), Ok(value) if value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes"))
}

fn root_dir() -> Option<PathBuf> {
    let manifest_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return None,
    };

    manifest_dir.parent().map(Path::to_path_buf)
}

fn local_artifacts_available_at(root_dir: &Path) -> bool {
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
