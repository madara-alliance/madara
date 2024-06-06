fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir = std::path::PathBuf::from(out_dir);
    std::fs::write(out_dir.join("target_os.txt"), std::env::var("CARGO_CFG_TARGET_OS").unwrap()).unwrap();
    std::fs::write(out_dir.join("target_arch.txt"), std::env::var("CARGO_CFG_TARGET_ARCH").unwrap()).unwrap();
    std::fs::write(out_dir.join("target_env.txt"), std::env::var("CARGO_CFG_TARGET_ENV").unwrap()).unwrap();
}
