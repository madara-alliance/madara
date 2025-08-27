fn main() {
    std::thread::sleep(std::time::Duration::from_secs(300));
    build_version::get_or_compile_artifacts(4).expect("Failed to load artifacts");
}
