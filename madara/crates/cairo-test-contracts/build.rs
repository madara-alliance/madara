fn main() {
    build_version::get_or_compile_artifacts(3).expect("Failed to load artifacts");
}
