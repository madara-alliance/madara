// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::{tests::common::*, types::jobs::types::{JobStatus, JobType}, worker::event_handler::triggers::{proving::ProcessingContext, update_state::UpdateStateJobTrigger, JobTrigger}};
//     use mockall::predicate::*;
//     use rstest::rstest;
//     use std::sync::Arc;

//     #[tokio::test]
//     async fn test_run_worker_with_pending_job_returns_early() {
//         let config = setup_config().await;
//         let trigger = UpdateStateJobTrigger;

//         // Mock database to return a pending state transition job
//         config
//             .database_mock()
//             .expect_get_latest_job_by_type()
//             .with(eq(JobType::StateTransition))
//             .returning(|_| Ok(Some(create_mock_job(JobStatus::Created))));

//         let result = trigger.run_worker(config).await;

//         assert!(result.is_ok());
//         // Should return early without creating new jobs
//     }

//     #[tokio::test]
//     async fn test_run_worker_no_blocks_to_process() {
//         let config = setup_config().await;
//         let trigger = UpdateStateJobTrigger;

//         // Mock no previous state transition jobs
//         config
//             .database_mock()
//             .expect_get_latest_job_by_type()
//             .with(eq(JobType::StateTransition))
//             .returning(|_| Ok(None));

//         config.database_mock().expect_get_earliest_failed_block_number().returning(|| Ok(None));

//         // Mock no unprocessed DA jobs
//         config.database_mock().expect_get_jobs_without_successor().returning(|_, _, _| Ok(vec![]));

//         let result = trigger.run_worker(config).await;

//         assert!(result.is_ok());
//     }

//     #[tokio::test]
//     async fn test_run_worker_successful_job_creation() {
//         let config = setup_config().await;
//         let trigger = UpdateStateJobTrigger;

//         setup_successful_workflow_mocks(&config).await;

//         let result = trigger.run_worker(config).await;

//         assert!(result.is_ok());
//     }

//     #[tokio::test]
//     async fn test_processing_context_check_pending_jobs_none() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config.database_mock().expect_get_latest_job_by_type().returning(|_| Ok(None));

//         let result = context.check_pending_jobs().await.unwrap();

//         assert!(result.is_none());
//     }

//     #[tokio::test]
//     async fn test_processing_context_check_pending_jobs_completed() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config
//             .database_mock()
//             .expect_get_latest_job_by_type()
//             .returning(|_| Ok(Some(create_mock_job(JobStatus::Completed))));

//         let result = context.check_pending_jobs().await.unwrap();

//         assert!(result.is_none());
//     }

//     #[tokio::test]
//     async fn test_processing_context_check_pending_jobs_pending() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config
//             .database_mock()
//             .expect_get_latest_job_by_type()
//             .returning(|_| Ok(Some(create_mock_job(JobStatus::Created))));

//         let result = context.check_pending_jobs().await.unwrap();

//         assert!(result.is_some());
//         assert!(result.unwrap().contains("pending update state job"));
//     }

//     #[tokio::test]
//     async fn test_determine_blocks_to_process_no_da_jobs() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config.database_mock().expect_get_latest_job_by_type().returning(|_| Ok(None));

//         config.database_mock().expect_get_earliest_failed_block_number().returning(|| Ok(None));

//         config.database_mock().expect_get_jobs_without_successor().returning(|_, _, _| Ok(vec![]));

//         let result = context.determine_blocks_to_process().await.unwrap();

//         assert!(result.is_none());
//     }

//     #[tokio::test]
//     async fn test_determine_blocks_to_process_with_failed_block_filter() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config.database_mock().expect_get_latest_job_by_type().returning(|_| Ok(None));

//         // Set failed block at 5
//         config.database_mock().expect_get_earliest_failed_block_number().returning(|| Ok(Some(5)));

//         // Return DA jobs with blocks 3, 4, 5, 6
//         config.database_mock().expect_get_jobs_without_successor().returning(|_, _, _| {
//             Ok(vec![create_mock_da_job("3"), create_mock_da_job("4"), create_mock_da_job("5"), create_mock_da_job("6")])
//         });

//         config.service_config_mock().expect_min_block_to_process().returning(|| 3);

//         let result = context.determine_blocks_to_process().await.unwrap();

//         // Should only process blocks 3, 4 (before failed block 5)
//         assert_eq!(result, Some(vec![3, 4]));
//     }

//     #[tokio::test]
//     async fn test_determine_blocks_to_process_all_blocks_after_failed() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config.database_mock().expect_get_latest_job_by_type().returning(|_| Ok(None));

//         // Set failed block at 3
//         config.database_mock().expect_get_earliest_failed_block_number().returning(|| Ok(Some(3)));

//         // Return DA jobs with blocks 5, 6, 7 (all after failed block)
//         config
//             .database_mock()
//             .expect_get_jobs_without_successor()
//             .returning(|_, _, _| Ok(vec![create_mock_da_job("5"), create_mock_da_job("6"), create_mock_da_job("7")]));

//         let result = context.determine_blocks_to_process().await.unwrap();

//         // Should return None as all blocks are at/after failed block
//         assert!(result.is_none());
//     }

//     #[tokio::test]
//     async fn test_validate_block_continuity_first_run_valid() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config.service_config_mock().expect_min_block_to_process().returning(|| 1);

//         let result = context.validate_block_continuity(&[1, 2, 3], None).await.unwrap();

//         assert!(result);
//     }

//     #[tokio::test]
//     async fn test_validate_block_continuity_first_run_invalid() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         config.service_config_mock().expect_min_block_to_process().returning(|| 1);

//         let result = context.validate_block_continuity(&[3, 4, 5], None).await.unwrap();

//         assert!(!result);
//     }

//     #[tokio::test]
//     async fn test_validate_block_continuity_continuation_valid() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         let result = context.validate_block_continuity(&[6, 7, 8], Some(5)).await.unwrap();

//         assert!(result);
//     }

//     #[tokio::test]
//     async fn test_validate_block_continuity_continuation_invalid() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         let result = context.validate_block_continuity(&[8, 9, 10], Some(5)).await.unwrap();

//         assert!(!result);
//     }

//     #[tokio::test]
//     async fn test_limit_batch_size_under_limit() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         let blocks = vec![1, 2, 3, 4, 5];
//         let result = context.limit_batch_size(blocks.clone());

//         assert_eq!(result, blocks);
//     }

//     #[tokio::test]
//     async fn test_limit_batch_size_over_limit() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         let blocks = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
//         let result = context.limit_batch_size(blocks);

//         assert_eq!(result, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
//         assert_eq!(result.len(), 10);
//     }

//     #[tokio::test]
//     async fn test_extract_block_numbers_valid_ids() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         let jobs = vec![create_mock_da_job("1"), create_mock_da_job("2"), create_mock_da_job("3")];

//         let result = context.extract_block_numbers(&jobs).unwrap();

//         assert_eq!(result, vec![1, 2, 3]);
//     }

//     #[tokio::test]
//     async fn test_extract_block_numbers_invalid_id() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();

//         let jobs = vec![create_mock_da_job("1"), create_mock_da_job("invalid"), create_mock_da_job("3")];

//         let result = context.extract_block_numbers(&jobs);

//         assert!(result.is_err());
//     }

//     #[tokio::test]
//     async fn test_build_state_metadata() {
//         let config = setup_config().await;
//         let context = ProcessingContext::new(config.clone()).await.unwrap();
//         let trigger = UpdateStateJobTrigger;

//         setup_metadata_mocks(&config).await;

//         let blocks = vec![1, 2];
//         let result = trigger.build_state_metadata(&blocks, &context).await.unwrap();

//         assert_eq!(result.blocks_to_settle, blocks);
//         assert!(!result.snos_output_paths.is_empty());
//         assert!(!result.program_output_paths.is_empty());
//         assert!(!result.blob_data_paths.is_empty());
//     }

//     // Helper functions
//     async fn setup_config() -> Arc<Config> {
//         // Return mock config - implementation depends on your test setup
//         todo!("Implement config setup")
//     }

//     fn create_mock_job(status: JobStatus) -> JobItem {
//         // Create mock job with given status
//         todo!("Implement mock job creation")
//     }

//     fn create_mock_da_job(internal_id: &str) -> JobItem {
//         // Create mock DA job with specific internal_id
//         todo!("Implement mock DA job creation")
//     }

//     async fn setup_successful_workflow_mocks(config: &Arc<Config>) {
//         // Setup all mocks for successful workflow
//         todo!("Implement successful workflow mocks")
//     }

//     async fn setup_metadata_mocks(config: &Arc<Config>) {
//         // Setup mocks for metadata collection
//         todo!("Implement metadata mocks")
//     }
// }
