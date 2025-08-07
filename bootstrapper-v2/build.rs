use std::fs;
use std::path::Path;

fn main() {
    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");

    const BOOTSTRAPPER_DIR: &str = "../build-artifacts/bootstrapper";
    const CONTRACTS_DIR: &str = "./contracts";

    // Copy build artifacts to the appropriate locations
    let copy_dirs = [
        (format!("{}/solidity/starkware", BOOTSTRAPPER_DIR), format!("{}/ethereum/src/starkware", CONTRACTS_DIR)),
        (format!("{}/solidity/third_party", BOOTSTRAPPER_DIR), format!("{}/ethereum/src/third_party", CONTRACTS_DIR)),
        (format!("{}/solidity/out", BOOTSTRAPPER_DIR), format!("{}/ethereum/out", CONTRACTS_DIR)),
        (format!("{}/cairo/target", BOOTSTRAPPER_DIR), format!("{}/madara/target", CONTRACTS_DIR)),
    ];

    for (src, dst) in copy_dirs.iter() {
        let src_path = Path::new(src);
        let dst_path = Path::new(dst);

        // Skipping if the src_path does not exist
        // Run `make artifacts` before to ensure artifacts are present
        if src_path.exists() {
            fs::create_dir_all(dst_path.parent().unwrap()).expect("Failed to get parent directory");
            fs_extra::dir::copy(
                src_path,
                dst_path.parent().unwrap(),
                &fs_extra::dir::CopyOptions::new().skip_exist(true),
            )
            .unwrap_or_else(|_| panic!("Failed to copy from {} to {}", src, dst));
        }
    }
}
