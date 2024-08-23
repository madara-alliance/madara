# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Added

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

- GitHub's coverage CI yml file for localstack and db testing.
- Orchestrator :Moved TestConfigBuilder to `config.rs` in tests folder.
- `.env` file requires two more variables which are queue urls for processing
  and verification.

## Removed

- `init_config` from all the tests.
- `fetch_from_test` argument

## Fixed

- Get Fact Info logic.
- Fixed state update worker logic as per the new implementation.
