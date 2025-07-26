use fs_extra;
use std::fs;
use std::path::Path;

fn main() {
    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");

    // Copy build artifacts to the appropriate locations
    let copy_dirs = [
        ("../build-artifacts/bootstrapper/solidity/starkware", "./contracts/ethereum/src/starkware"),
        ("../build-artifacts/bootstrapper/solidity/third_party", "./contracts/ethereum/src/third_party"),
        ("../build-artifacts/bootstrapper/solidity/out", "./contracts/ethereum/out"),
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
            .expect(&format!("Failed to copy from {} to {}", src, dst));
        }
    }
}
