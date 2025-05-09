use std::path::PathBuf;

fn main() {
    let cairo_project_dir = PathBuf::from("../../../cairo/").canonicalize().unwrap();
    println!("cargo::rerun-if-changed={}", cairo_project_dir.join("src").display());
    println!("cargo::rerun-if-changed={}", cairo_project_dir.join("Scarb.toml").display());
    // Note: scarb build updates the Scarb.lock file, and there is no `--locked` arg to avoid
    //  that, so we can't use rerun-if-change on it :/
    // println!("cargo::rerun-if-changed={}", cairo_project_dir.join("Scarb.lock").display());
    let exit_status = std::process::Command::new("scarb")
        .current_dir(cairo_project_dir)
        .args(["build"])
        .spawn()
        .expect("Scarb needs to be installed in order to compile the test contracts")
        .wait()
        .unwrap();
    if !exit_status.success() {
        panic!(
            "`scarb build` command returned with a non-zero exit code: {}",
            exit_status.code().expect("`scarb build` command was stopped by a signal`")
        );
    }
}
