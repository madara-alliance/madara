# Madara Orchestrator

The Madara orchestrator is designed to be an additional service which runs in parallel to Madara and handles

1. publishing data to the respective DA layer
2. running SNOS and submitting jobs to the prover
3. updating the state on Cairo core contracts

As a v1, the orchestrator handles the DA publishing. The architecture for the same is as follows

![orchestrator_da_sequencer_diagram](./docs/orchestrator_da_sequencer_diagram.png)