fn main() {
    // We just use this to trigger re-compilation in case the build artifacts change
    println!("cargo:rerun-if-changed=../build_artifacts/");
}
