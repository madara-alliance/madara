use std::path::{Path, PathBuf};

use sc_cli::{Error, Result, SubstrateCli};
use sc_service::BasePath;
use sha3::{Digest, Sha3_256};
use url::Url;

use crate::chain_spec::GENESIS_ASSETS_DIR;
use crate::cli::Cli;
use crate::configs::FileInfos;
use crate::{configs, constants};

/// Define a way to retrieve an index.json file

/// The index.json must follow the format of the official index.json
/// (https://github.com/kasarlabs/deoxys/blob/main/configs/index.json)
/// Where the `sha3_256` and `url` fields are optional
#[derive(Debug, clap::Args)]
#[group(required = true, multiple = false)]
pub struct SetupSource {
    /// Download an index.json file for an url
    #[clap(
        long,
        conflicts_with="from_local",
        value_hint=clap::ValueHint::Url,
        // This combination of properties allow us to use a default value only if the arg is passed without value.
        // If it is not passed at all, no default value is used.
        // See: https://docs.rs/clap/latest/clap/struct.Arg.html#method.default_missing_value
        num_args = 0..=1, require_equals=true, default_missing_value = Some(constants::DEFAULT_CONFIGS_URL)
    )]
    pub from_remote: Option<String>,

    /// Copy an index.json file for an url
    #[clap(long, conflicts_with = "from_remote")]
    pub from_local: Option<String>,
}

enum ConfigSource {
    Local(PathBuf),
    Remote(Url),
}

impl ConfigSource {
    fn display(&self, child_path: Option<Vec<&str>>) -> Result<String> {
        let string = match self {
            ConfigSource::Local(path_buf) => {
                let mut full = path_buf.clone();

                if let Some(childs) = child_path {
                    for child in childs {
                        full = full.join(child);
                    }
                }

                full.display().to_string()
            }
            ConfigSource::Remote(url) => {
                let mut full = url.clone();

                if let Some(childs) = child_path {
                    for child in childs {
                        full = full.join(child).map_err(|e| Error::Application(Box::new(e)))?;
                    }
                }

                full.to_string()
            }
        };

        Ok(string)
    }

    fn load_config(&self) -> Result<Vec<u8>> {
        let configs: Vec<u8> = match self {
            ConfigSource::Local(source_configs_path) => {
                let index_file_path = source_configs_path.join("index.json");
                std::fs::read(index_file_path).map_err(|e| Error::Application(Box::new(e)))?
            }
            ConfigSource::Remote(source_configs_url) => {
                println!("Fetching chain config from '{}'", &source_configs_url);

                // Query, deserialize and copy it
                let response =
                    reqwest::blocking::get(source_configs_url.clone()).map_err(|e| Error::Application(Box::new(e)))?;
                response.bytes().map_err(|e| Error::Application(Box::new(e)))?.into()
            }
        };

        Ok(configs)
    }

    fn load_asset(&self, asset: &FileInfos) -> Result<Vec<u8>> {
        let file_as_bytes = match self {
            ConfigSource::Local(source_config_dir_path) => {
                let asset_path = &source_config_dir_path.join(GENESIS_ASSETS_DIR).join(&asset.name);
                if !asset_path.exists() {
                    return Err(format!("Source file '{}' does not exist", asset_path.display()).into());
                }
                std::fs::read(asset_path)?
            }
            ConfigSource::Remote(source_configs_dir_url) => {
                let full_url = source_configs_dir_url
                    .join(GENESIS_ASSETS_DIR)
                    .map_err(|e| Error::Application(Box::new(e)))?
                    .join(&asset.name)
                    .map_err(|e| Error::Application(Box::new(e)))?;

                let response = reqwest::blocking::get(full_url.clone()).map_err(|e| Error::Application(Box::new(e)))?;
                response.bytes().map_err(|e| Error::Application(Box::new(e)))?.into()
            }
        };

        Ok(file_as_bytes)
    }
}

fn write_content_to_disk<T: AsRef<[u8]>>(config_content: T, dest_config_file_path: &Path) -> Result<()> {
    std::fs::create_dir_all(
        dest_config_file_path.parent().expect("dest_config_file_path should be the path to a file, not a directory"),
    )?;
    let mut dest_file = std::fs::File::create(dest_config_file_path)?;
    let mut reader = std::io::Cursor::new(config_content);
    std::io::copy(&mut reader, &mut dest_file)?;

    Ok(())
}
