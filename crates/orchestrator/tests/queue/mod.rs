use super::init_valid_config;
use orchestrator::config::Config;
use rstest::*;

#[rstest]
#[tokio::test]
async fn test_abc(
    #[future] init_valid_config: &Config
) {
    init_valid_config.await;
    todo!("setting up the structure before writing the tests");
}