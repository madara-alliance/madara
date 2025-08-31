mod cli;
pub mod config;
pub mod utils;
mod setup {
    pub mod base_layer;
    pub mod madara;
}

use clap::Parser;
use cli::{CliArgs, Commands};

use anyhow::{Context, Result};
use config::{BaseConfigOuter, MadaraConfigOuter};
use setup::madara::MadaraSetup;
use std::fs::File;

fn main() -> Result<()> {
    dotenvy::from_filename_override(".env")?;
    let args = CliArgs::parse();

    match args.command {
        Commands::SetupBase(setup_base) => {
            let config: BaseConfigOuter = serde_json::from_reader(File::open(setup_base.config_path).context(
                "Failed to read base layer config file.
                Make sure the path exists and is readable",
            )?)
            .context("Failed to deserialize base layer config file")?;

            let mut base_layer_setup = config
                .get_base_layer_setup(setup_base.private_key, &setup_base.addresses_output_path)
                .context("Failed to initialise base layer config")?;

            base_layer_setup.init().context("Failed to initialise the base layer setup")?;
            base_layer_setup.setup().context("Failed to setup base layer setup")?;
        }

        Commands::SetupMadara(setup_madara) => {
            let madara_config: MadaraConfigOuter = serde_json::from_reader(File::open(&setup_madara.config_path)?)?;
            let base_layer_config: BaseConfigOuter = serde_json::from_reader(File::open(setup_madara.config_path)?)?;

            let madara_setup = MadaraSetup::new(madara_config.madara, setup_madara.private_key);
            let base_layer_setup = base_layer_config
                .get_base_layer_setup(setup_madara.base_layer_private_key, &setup_madara.base_addresses_path)?;

            madara_setup.init().context("Failed to initialise the madara setup")?;
            madara_setup
                .setup(&setup_madara.base_addresses_path, &setup_madara.output_path)
                .context("Failed to setup madara setup")?;

            base_layer_setup.post_madara_setup(&setup_madara.output_path).context("Failed to post madara setup")?;
        }
    }

    Ok(())
}
