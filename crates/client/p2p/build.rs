use std::io::Result;
fn main() -> Result<()> {
    let files: Vec<_> = std::fs::read_dir("proto")?.map(|entry| entry.map(|e| e.path())).collect::<Result<_>>()?;
    prost_build::compile_protos(&files, &["proto/"])?;
    Ok(())
}
