use rstest::*;

use crate::common::{
    get_or_init_config,
    default_job_item,
};

use orchestrator::{
    config::Config,
    jobs::{
        da_job::DaJob,
        types::{JobItem, JobType},
        Job
    },
};

#[rstest]
#[tokio::test]
async fn test_create_job(
    #[future] get_or_init_config: &Config
) {
    let config = get_or_init_config.await;
    let job = DaJob.create_job(&config, String::from("0")).await;
    assert!(job.is_ok());

    let job = job.unwrap();
    
    let job_type = job.job_type;
    assert_eq!(job_type, JobType::DataSubmission);
    assert_eq!(job.metadata.values().len(), 0, "metadata should be empty");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
// #[should_panic]
#[tokio::test]
async fn test_verify_job(
    #[future] #[from(get_or_init_config)] config: &Config,
    default_job_item: JobItem,
) {
    let config = config.await;
    assert!(DaJob.verify_job(config, &default_job_item).await.is_err());
}