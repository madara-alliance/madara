use crate::config::MadaraConfig;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
}

impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig, _private_key: String) -> Self {
        Self { rpc_url: madara_config.rpc_url }
    }

    pub fn init(&self) -> color_eyre::Result<()> {
        Ok(())
    }

    pub fn setup(&self) -> color_eyre::Result<()> {
        Ok(())
    }

    pub fn post_madara_setup(&self) -> color_eyre::Result<()> {
        Ok(())
    }
}
