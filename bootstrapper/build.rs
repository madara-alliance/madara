fn main() {
    println!("cargo:warning=bootstrapper v1 is archived and unmaintained; use bootstrapper-v2 instead");
    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");
}
