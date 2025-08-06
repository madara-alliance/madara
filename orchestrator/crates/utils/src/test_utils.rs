use color_eyre::eyre::{eyre, Result};
use reqwest::Client;
use rstest::fixture;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use tempfile::TempDir;
use tracing::{debug, info, warn};
use url::Url;

pub const TEST_DATA_BASE_URL_ENV: &str = "MADARA_ORCHESTRATOR_TEST_DATA_BASE_URL";

/// Simple test data setup function
/// Downloads files from the given URLs and optionally decompresses them per file
/// Takes a vector of (url, should_decompress) tuples
/// Returns a TempDir containing the test data files
#[fixture]
pub async fn setup_test_data(#[default(vec![])] files: Vec<(&str, bool)>) -> Result<TempDir> {
    let base_url: Url = match std::env::var(TEST_DATA_BASE_URL_ENV) {
        Ok(mut url) => {
            if !url.ends_with('/') {
                url.push('/');
            }
            Url::from_str(&url)?
        }
        Err(_) => {
            return Err(eyre!("{} environment variable is not set", TEST_DATA_BASE_URL_ENV));
        }
    };

    info!("Test data base URL: {}", base_url);

    // Create a temporary directory
    let temp_dir = tempfile::tempdir()?;

    let client = Client::new();

    // Download each file
    for (path, should_decompress) in files {
        let filename = extract_filename_from_url(path)?;
        let file_path = temp_dir.path().join(&filename);

        if !file_path.exists() {
            debug!("Downloading {}", filename);
            download_file(&client, base_url.join(path)?.as_str(), &file_path).await?;

            debug!("Downloaded {}", filename);

            if should_decompress {
                decompress_file(&file_path)?;
            }
        }
    }

    info!("Test data setup complete");

    Ok(temp_dir)
}

/// Extract filename from URL
fn extract_filename_from_url(url: &str) -> Result<String> {
    let path = url.split('/').next_back().ok_or(eyre!("Invalid URL: no filename found"))?;

    // Remove query parameters if any
    let filename = path.split('?').next().unwrap_or(path);

    if filename.is_empty() {
        return Err(eyre!("Invalid URL: empty filename"));
    }

    Ok(filename.to_string())
}

/// Download a file from URL to the specified path
async fn download_file(client: &Client, url: &str, dest_path: &Path) -> Result<()> {
    info!("Downloading {} to {}", url, dest_path.display());

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(eyre!("Failed to download: HTTP {}", response.status()));
    }

    let bytes = response.bytes().await?;
    fs::write(dest_path, bytes)?;
    Ok(())
}

/// Decompress a file based on its extension
fn decompress_file(file_path: &Path) -> Result<()> {
    info!("Decompressing {}", file_path.display());
    let filename =
        file_path.file_name().and_then(|name| name.to_str()).ok_or(eyre!("Invalid filename"))?.to_lowercase();

    if filename.ends_with(".zip") {
        decompress_zip(file_path)?;
    } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        decompress_tar_gz(file_path)?;
    } else {
        warn!("Warning: Only .zip and .tar.gz/.tgz files are supported for decompression, skipping");
    }

    Ok(())
}

/// Decompress ZIP files
fn decompress_zip(file_path: &Path) -> Result<()> {
    let file = fs::File::open(file_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let extract_dir = file_path.parent().unwrap();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let output_path = extract_dir.join(file.name());

        if file.name().ends_with('/') {
            // Directory
            fs::create_dir_all(&output_path)?;
        } else {
            // File
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output_file = fs::File::create(&output_path)?;
            std::io::copy(&mut file, &mut output_file)?;
        }
    }

    info!("Extracted ZIP: {}", file_path.display());
    Ok(())
}

/// Decompress TAR.GZ files
fn decompress_tar_gz(file_path: &Path) -> Result<()> {
    use flate2::read::GzDecoder;

    let file = fs::File::open(file_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let extract_dir = file_path.parent().unwrap();

    archive.unpack(extract_dir)?;

    info!("Extracted TAR.GZ: {}", file_path.display());
    Ok(())
}
