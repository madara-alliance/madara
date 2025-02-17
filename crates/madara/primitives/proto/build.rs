fn main() -> std::io::Result<()> {
    let files: Vec<_> = collect_files("starknet-p2p-specs/p2p/proto/", "proto")?;

    prost_build::Config::new()
        .extern_path(".Felt252", "crate::proto::model_primitives::Felt252")
        .extern_path(".Hash", "crate::proto::model_primitives::Hash")
        .extern_path(".EthereumAddress", "crate::proto::model_primitives::EthereumAddress")
        .extern_path(".Hash256", "crate::proto::model_primitives::Hash256")
        .extern_path(".Address", "crate::proto::model_primitives::Address")
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(&files, &["starknet-p2p-specs/"])?;
    Ok(())
}

/// Recursively collects all files in a directory.
///
/// This function will keep exploring sub-directories until it has reached the
/// bottom of the file tree.
fn collect_files(path: impl AsRef<std::path::Path>, ext: &str) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut files = vec![];

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            files.extend(collect_files(path, ext)?);
        } else if path.extension().expect("file with no extension") == ext {
            files.push(path)
        }
    }

    std::io::Result::Ok(files)
}
