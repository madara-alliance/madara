use anyhow::Context;

pub fn save_addresses_to_file(addresses_json: String, file_path: &str) -> anyhow::Result<()> {
    // Ensure parent directories exist before writing the file
    if let Some(parent_dir) = std::path::Path::new(&file_path).parent() {
        std::fs::create_dir_all(parent_dir)
            .with_context(|| format!("Failed to create parent directories for: {}", &file_path))?;
    }

    std::fs::write(&file_path, &addresses_json)
        .with_context(|| format!("Failed to write addresses to: {}", &file_path))?;

    Ok(())
}
