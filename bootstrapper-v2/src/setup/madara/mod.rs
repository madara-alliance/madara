use crate::config::MadaraConfig;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
}

impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig) -> Self {
        Self { rpc_url: madara_config.rpc_url }
    }

    pub fn init(&self) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn setup(&self) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn post_madara_setup(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
