fn main() {
    println!("cargo::rerun-if-changed=../../../cairo/src");
    println!("cargo::rerun-if-changed=../../../cairo/Scarb.toml");
    println!("cargo::rerun-if-changed=../../../cairo/Scarb.lock");
    std::process::Command::new("scarb")
        .args(["build"])
        .spawn()
        .expect("Scarb needs to be installed in order to compile the devnet contracts.")
        .wait()
        .unwrap();
}
