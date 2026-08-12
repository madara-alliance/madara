use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_PATH: &str = "src/contracts/paradex/supported_contracts.json";

fn main() {
    println!("cargo:rerun-if-changed={MANIFEST_PATH}");

    let manifest_path = Path::new(MANIFEST_PATH);
    let manifest_dir = manifest_path.parent().expect("supported contracts manifest must have a parent");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest_path).expect("failed to read supported_contracts.json"))
            .expect("supported_contracts.json must be valid JSON");

    for contract in manifest.as_object_mut().expect("supported_contracts.json must contain a JSON object").values_mut()
    {
        let contract = contract.as_object_mut().expect("each supported contract must be a JSON object");
        let Some(functions_file) = contract.remove("supported_functions_file") else {
            continue;
        };
        let functions_file = functions_file.as_str().expect("supported_functions_file must be a string");
        let functions_path = manifest_dir.join(functions_file);
        println!("cargo:rerun-if-changed={}", functions_path.display());

        let functions: serde_json::Value = serde_json::from_slice(
            &fs::read(&functions_path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", functions_path.display())),
        )
        .unwrap_or_else(|error| panic!("{} must be valid JSON: {error}", functions_path.display()));
        assert!(
            functions.as_array().is_some_and(|items| items.iter().all(serde_json::Value::is_string)),
            "{} must contain an array of function names",
            functions_path.display()
        );
        assert!(
            contract.insert("supported_functions".to_string(), functions).is_none(),
            "supported_functions and supported_functions_file cannot both be set"
        );
    }

    let output_path =
        PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR must be set")).join("supported_contracts.json");
    fs::write(output_path, serde_json::to_vec_pretty(&manifest).expect("failed to serialize supported contracts"))
        .expect("failed to write generated supported contracts manifest");
}
