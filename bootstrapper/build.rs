fn main() {
    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");
}
