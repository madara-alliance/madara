# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Added

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
- Tests for DA job.

## Changed

- GitHub's coverage CI yml file for localstack and db testing.
- Orchestrator :Moved TestConfigBuilder to `config.rs` in tests folder.
- `.env` file requires two more variables which are queue urls for processing
  and verification.
- Shifted Unit tests to test folder for DA job.

## Removed

- `fetch_from_test` argument

## Fixed
