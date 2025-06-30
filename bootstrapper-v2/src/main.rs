mod cli;
pub mod config;
mod setup {
    pub mod base_layer;
    pub mod madara;
}

use clap::Parser;
use cli::{CliArgs, Commands};
use color_eyre::Result;
use config::Config;
use setup::madara::MadaraSetup;
use std::fs::File;

fn main() -> Result<()> {
    color_eyre::install()?;

    dotenvy::from_filename_override("../.env")?;
    let args = CliArgs::parse();

    match args.command {
        Commands::SetupBase(setup_base) => {
            let config: Config = serde_json::from_reader(File::open(setup_base.config_path)?)?;
            config.validate()?;

            let private_key = dotenvy::var("BASE_LAYER_PRIVATE_KEY")?;
            let base_layer_setup =
                config.get_base_layer_setup(private_key).map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            base_layer_setup
                .init()
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            base_layer_setup
                .setup()
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        }
        Commands::SetupMadara(setup_madara) => {
            let config: Config = serde_json::from_reader(File::open(setup_madara.config_path)?)?;
            config.validate()?;
            let madara_setup = MadaraSetup::new(config.madara.unwrap());
            madara_setup
                .init()
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            madara_setup
                .setup()
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
            madara_setup
                .post_madara_setup()
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))
                .map_err(|e| color_eyre::eyre::eyre!("{}", e))?;
        }
    }

    Ok(())
}
