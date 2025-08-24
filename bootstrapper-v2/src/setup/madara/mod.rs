use crate::config::MadaraConfig;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MadaraSetup {
    rpc_url: String,
}

#[allow(unused_variables)]
impl MadaraSetup {
    pub fn new(madara_config: MadaraConfig, _private_key: String) -> Self {
        Self { rpc_url: madara_config.rpc_url }
    }

    pub fn init(&self) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn setup(&self, base_addresses_path: &str, madara_addresses_path: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
