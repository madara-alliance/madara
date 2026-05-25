fn main() {
    let cargo_lock_path = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .ancestors()
        .map(|path| path.join("Cargo.lock"))
        .find(|path| path.exists())
        .expect("workspace Cargo.lock must exist");
    let cargo_lock = std::fs::read_to_string(&cargo_lock_path).expect("workspace Cargo.lock must be readable");
    let cairo_native_version = cargo_lock
        .split("[[package]]")
        .find(|package| package.contains("name = \"cairo-native\""))
        .and_then(|package| package.lines().find_map(|line| line.trim().strip_prefix("version = \"")))
        .and_then(|version| version.strip_suffix('"'))
        .expect("Cargo.lock must contain cairo-native");

    println!("cargo:rustc-env=MADARA_CAIRO_NATIVE_VERSION={cairo_native_version}");
    println!("cargo:rerun-if-changed={}", cargo_lock_path.display());
}
