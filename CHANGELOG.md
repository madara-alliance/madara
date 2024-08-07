# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Added

- implemented DA worker.
- Function to calculate the kzg proof of x_0.
- Tests for updating the state.
- Function to update the state and publish blob on ethereum in state update job.
- Fixtures for testing.
- Added basic rust-toolchain support.

## Changed

- GitHub's coverage CI yml file for localstack and db testing.
- Orchestrator :Moved TestConfigBuilder to `config.rs` in tests folder.

## Removed

- `fetch_from_test` argument

## Fixed
