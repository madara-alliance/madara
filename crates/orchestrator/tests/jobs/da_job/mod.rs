use rstest::*;

use crate::common::{
    get_or_init_config,
    default_job_item,
    custom_job_item,
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
    #[future] #[from(get_or_init_config)] config: &Config,
) {
    let config = config.await;
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
    #[from(default_job_item)] job_item: JobItem,
) {
    let config = config.await;

    assert!(DaJob.verify_job(config, &job_item).await.is_err());
}

#[rstest]
#[should_panic]
#[tokio::test]
async fn test_process_job(
    #[future] #[from(get_or_init_config)] config: &Config,
    #[from(default_job_item)] job_item: JobItem,
) {
    let config = config.await;
    let job_item = custom_job_item(job_item, String::from("1"));

    let result = DaJob.process_job(config, &job_item).await;
    assert!(result.is_ok());
}

#[rstest]
fn test_max_process_attempts() {
    assert_eq!(DaJob.max_process_attempts(), 1);
}

#[rstest]
fn test_max_verification_attempts() {
    assert_eq!(DaJob.max_verification_attempts(), 3);
}

#[rstest]
fn test_verification_polling_delay_seconds() {
    assert_eq!(DaJob.verification_polling_delay_seconds(), 60);
}