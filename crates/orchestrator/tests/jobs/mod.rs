use super::common::get_or_init_config;
use orchestrator::config::Config;
use rstest::*;

#[rstest]
#[tokio::test]
async fn test_abc(
    #[future]
    #[with( String::from("http://localhost:9944") )]
    get_or_init_config: &Config
) {
    get_or_init_config.await;
    todo!("setting up the structure before writing the tests");
}