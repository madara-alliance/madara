use anyhow::Context;
use bootstrapper_v2::cli::{CliArgs, Commands};
use bootstrapper_v2::config::{BaseConfigOuter, MadaraConfigOuter};
use bootstrapper_v2::setup::madara::MadaraSetup;
use bootstrapper_v2::BootstrapperResult;
use clap::Parser;
use env_logger::Env;
use std::fs::File;

#[tokio::main]
async fn main() -> BootstrapperResult<()> {
    dotenvy::from_filename_override(".env")?;
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let args = CliArgs::parse();

    match args.command {
        Commands::SetupBase(setup_base) => {
            let config: BaseConfigOuter = serde_json::from_reader(File::open(setup_base.config_path)?)?;

            let mut base_layer_setup =
                config.get_base_layer_setup(setup_base.private_key, &setup_base.addresses_output_path);

            base_layer_setup.init().await.context("Failed to initialise the base layer setup")?;
            base_layer_setup.setup().await.context("Failed to setup base layer setup")?;
        }

        Commands::SetupMadara(setup_madara) => {
            let madara_config: MadaraConfigOuter = serde_json::from_reader(File::open(&setup_madara.config_path)?)?;
            let base_layer_config: BaseConfigOuter = serde_json::from_reader(File::open(setup_madara.config_path)?)?;

            let mut madara_setup = MadaraSetup::new(madara_config.madara);
            let mut base_layer_setup = base_layer_config
                .get_base_layer_setup(setup_madara.base_layer_private_key.clone(), &setup_madara.base_addresses_path);

            madara_setup
                .init(&setup_madara.private_key, &setup_madara.output_path)
                .await
                .context("Failed to initialise the madara setup")?;
            madara_setup
                .setup(&setup_madara.base_addresses_path, &setup_madara.output_path)
                .await
                .context("Failed to setup madara setup")?;

            base_layer_setup
                .post_madara_setup(&setup_madara.output_path)
                .await
                .context("Failed to complete post madara setup")?;
        }
    }

    Ok(())
}
