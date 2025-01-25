# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Added

- ci: linters added
- readme: setup instructions added
- Added : Grafana dashboard
- tests: http_client tests added
- Added Atlantic proving service integration
- setup functions added for cloud and db
- Added cli args support for all the services
- Setup functions added for cloud and db
- panic handling in process job
- upgrade ETH L1 bridge for withdrawals to work
- added makefile and submodules
- Endpoints for triggering processing and verification jobs
- Add multiple queues for processing and verification based on job type
- added logs
- added MongoDB migrations using nodejs
- added dockerfile
- `SnosJob` implementation and e2e
- Telemetry tracing and metrics.
- e2e flow test
- database timestamps
- alerts module.
- Tests for Settlement client.
- Worker queues to listen for trigger events.
- Tests for prover client.
- Added Rust Cache for Coverage Test CI.
- support for fetching PIE file from storage client in proving job.
- added coveralls support
- moved mongodb serde behind feature flag
- implemented DA worker.
- Function to calculate the kzg proof of x_0.
- Tests for updating the state.
- Function to update the state and publish blob on ethereum in state update job.
- Tests for job handlers in orchestrator/src/jobs/mod.rs.
- Fixtures for testing.
- Basic rust-toolchain support.
- `AWS_DEFAULT_REGION="localhost"` var. in .env.test for omniqueue queue testing.
- Added basic rust-toolchain support.
- Implement DL queue for handling failed jobs.
- Added tests for state update job.
- Tests for DA job.
- Added generalized errors for Jobs : JobError.
- Database tests

## Changed

- refactor: expect removed and added error wraps
- refactor: Readme and .env.example
- refactor: http_mock version updated
- refactor: prover-services renamed to prover-clients
- refactor: update json made generic to update any json file
- refactor: makefile updated as per bootstraper changes
- removed error return in case of JobAlreadyExists in `create_job` function
- update_job returns the updated job item
- made create_job atomic to avoid race conditions
- handle jobs in tokio tasks
- handle workers in tokio tasks
- cleaned .env.example and .env.test files
- bumped snos and downgraded rust to match SNOS rust version
- Bumped dependencies, and associated api changes done
- ethereum DA client builder
- AWS config built from TestConfigBuilder.
- Better TestConfigBuilder, with sync config clients.
- Drilled Config, removing dirty global reads.
- settings provider
- refactor AWS config usage and clean .env files
- GitHub's coverage CI yml file for localstack and db testing.
- Orchestrator :Moved TestConfigBuilder to `config.rs` in tests folder.
- `.env` file requires two more variables which are queue urls for processing
  and verification.

## Removed

- docker-compose removed
- revert CI changes from settlement client PR.
- `init_config` from all the tests.
- `fetch_from_test` argument

## Fixed

- refactor: instrumentation
- `is_worker_enabled` status check moved from `VerificationFailed` to `Failed`
- refactor: static attributes for telemetry
- refactor: aws setup for Event Bridge
- refactor: RUST_LOG filtering support
- refactor: cargo.toml files cleaned
- blob data formation process from state update
- OTEL config refactor
- indexing for get_jobs_without_successor
- wait for transaction logic in ethereum settlement client
- y_0 point evaluation in build kzg proof for ethereum settlement
- fixed metrics name, signoz dashboard.
- fixes logs based on RUST_LOG
- fixes after sepolia testing
- all failed jobs should move to failed state
- Fixes all unwraps() in code to improve error logging
- Simplified Update_Job for Database.
- Simplified otel setup.
- Added new_with_settings to SharpClient.
- Calculate root hash logic and added a simple test for it.
- Cargo.toml dependency reorg.
- Get Fact Info logic.
- Fixed state update worker logic as per the new implementation.
