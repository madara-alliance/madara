pub mod ethereum;
pub mod starknet;

use anyhow::Result;

#[allow(dead_code)]
pub trait BaseLayerSetupTrait {
    /// This function does prerequisite setup for running the base layer setup.
    /// It should be called before the base layer setup.
    fn init(&mut self, addresses_output_path: String) -> Result<()>;
    fn setup(&self) -> Result<()>;
    fn post_madara_setup(&self) -> Result<()>;
}
