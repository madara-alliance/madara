use bootstrapper_v2::cli::{CliArgs, Commands};
use bootstrapper_v2::config::{BaseConfigOuter, MadaraConfigOuter};
use bootstrapper_v2::setup::madara::MadaraSetup;
use bootstrapper_v2::utils::DeployedAddresses;
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
                config.get_base_layer_setup(setup_base.private_key, &setup_base.addresses_output_path)?;

            base_layer_setup.init().await?;
            base_layer_setup.setup().await?;
        }

        Commands::SetupMadara(setup_madara) => {
            // Notice the same config path is used for both madara and base layer
            // This is because `serialization` can be done to both the types, ignoring the unknown fields.
            // This is useful as we can use the same config file for both madara and base layer.
            let madara_config: MadaraConfigOuter = serde_json::from_reader(File::open(&setup_madara.config_path)?)?;
            let base_layer_config: BaseConfigOuter = serde_json::from_reader(File::open(&setup_madara.config_path)?)?;

            let mut madara_setup = MadaraSetup::new(madara_config.madara);
            let mut base_layer_setup = base_layer_config
                .get_base_layer_setup(setup_madara.base_layer_private_key.clone(), &setup_madara.base_addresses_path)?;

            madara_setup.init(&setup_madara.private_key, &setup_madara.output_path).await?;
            madara_setup.setup(&setup_madara.base_addresses_path, &setup_madara.output_path).await?;

            // Step 1: post_madara_setup (setL2Bridge + enrollTokenBridge)
            base_layer_setup.post_madara_setup(&setup_madara.output_path).await?;

            // Step 2: Get the enrolled L2 token (polling L2)
            // Read addresses from the output files
            let base_addresses = DeployedAddresses::from_file(&setup_madara.base_addresses_path)?;
            let madara_addresses = DeployedAddresses::from_file(&setup_madara.output_path)?;

            let l1_token_address = base_addresses.get_address("l1Token").ok_or_else(|| {
                bootstrapper_v2::error::BootstrapperError::Other("l1Token not found in base addresses".into())
            })?;
            let l2_token_bridge = madara_addresses.get_address("l2_token_bridge").ok_or_else(|| {
                bootstrapper_v2::error::BootstrapperError::Other("l2_token_bridge not found in madara addresses".into())
            })?;

            let l2_fee_token = madara_setup
                .get_enrolled_l2_token(
                    l1_token_address,
                    l2_token_bridge,
                    300,  // 5 min timeout
                    5000, // 5 sec interval
                )
                .await?;

            // Save the fee token to Madara addresses file
            madara_setup.insert_address(
                bootstrapper_v2::setup::madara::DeployedContract::L2FeeToken,
                l2_fee_token,
            );
            madara_setup.save_madara_addresses(&setup_madara.output_path)?;

            // Step 3: Compare with configured fee token (warning only)
            // Read configured fee token from config
            let config_content = std::fs::read_to_string(&setup_madara.config_path)?;
            let config: serde_json::Value = serde_json::from_str(&config_content)?;

            if let Some(configured_fee_token_str) =
                config["base_layer"]["config_hash_config"]["madara_fee_token"].as_str()
            {
                if let Ok(configured_fee_token) = starknet::core::types::Felt::from_hex(configured_fee_token_str) {
                    if l2_fee_token != configured_fee_token {
                        log::warn!(
                            "Fee token mismatch: configured={:#x}, deployed={:#x}. Using deployed token for config hash.",
                            configured_fee_token,
                            l2_fee_token
                        );
                    }
                }
            }

            // Step 4: Verify/update config hash
            let core_contract_address = base_addresses.get_address("coreContract").ok_or_else(|| {
                bootstrapper_v2::error::BootstrapperError::Other("coreContract not found in base addresses".into())
            })?;

            base_layer_setup.verify_update_config_hash(&format!("{:#x}", l2_fee_token), core_contract_address).await?;

            log::info!("Post-Madara setup completed successfully!");
        }
    }

    Ok(())
}
