mod cli;
pub mod config;
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
            let config: BaseConfigOuter = serde_json::from_reader(File::open(setup_base.config_path)?)?;

            let mut base_layer_setup = config.get_base_layer_setup(setup_base.private_key)?;

            base_layer_setup
                .init(setup_base.addresses_output_path)
                .context("Failed to initialise the base layer setup")?;
            base_layer_setup.setup().context("Failed to setup base layer setup")?;
        }

        Commands::SetupMadara(setup_madara) => {
            let config: MadaraConfigOuter = serde_json::from_reader(File::open(setup_madara.config_path)?)?;
            let madara_setup = MadaraSetup::new(config.madara, setup_madara.private_key);

            madara_setup.init().context("Failed to initialise the madara setup")?;
            madara_setup.setup().context("Failed to setup madara setup")?;

            madara_setup.post_madara_setup().context("Failed to post madara setup")?;
        }
    }

    Ok(())
}
