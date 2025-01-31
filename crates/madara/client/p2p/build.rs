use std::io::Result;
fn main() -> Result<()> {
    let files: Vec<_> = std::fs::read_dir("proto")?.map(|entry| entry.map(|e| e.path())).collect::<Result<_>>()?;
    prost_build::Config::new()
        .extern_path(".Felt252", "crate::model_primitives::Felt252")
        .extern_path(".Hash", "crate::model_primitives::Hash")
        .extern_path(".EthereumAddress", "crate::model_primitives::EthereumAddress")
        .extern_path(".Hash256", "crate::model_primitives::Hash256")
        .extern_path(".Address", "crate::model_primitives::Address")
        // .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(&files, &["proto/"])?;
    Ok(())
}
