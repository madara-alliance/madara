# Madara Orchestrator

The Madara orchestrator is designed to be an additional service which runs in
parallel to Madara and handles

1. publishing data to the respective DA layer
2. running SNOS and submitting jobs to the prover
3. updating the state on Cairo core contracts

The tentative flow of the orchestrator looks like this but this is subject to
change as we learn more about external systems and the constraints involved.

![orchestrator_da_sequencer_diagram](./docs/orchestrator_da_sequencer_diagram.png)

## Testing

- Files needed for tests can be fetched through s3 :

  ```shell
    wget -P ./crates/prover-services/sharp-service/tests/artifacts https://madara-orchestrator-sharp-pie.s3.amazonaws.com/238996-SN.zip
  ```

- To run all the tests :

  ```shell
    cargo llvm-cov nextest --release --lcov --output-path lcov.info --test-threads=1
  ```
