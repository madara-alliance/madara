# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Added

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

- revert CI changes from settlement client PR.
- `init_config` from all the tests.
- `fetch_from_test` argument

## Fixed

- Calculate root hash logic and added a simple test for it.
- Cargo.toml dependency reorg.
- Get Fact Info logic.
- Fixed state update worker logic as per the new implementation.
