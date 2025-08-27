fn main() {
    std::thread::sleep(std::time::Duration::from_secs(1500));
    build_version::get_or_compile_artifacts(1).expect("Failed to load artifacts");
}
