//! Madara end-to-end tests. This is responsible for validating functionality across RPC endpoints,
//! transaction flows, and node synchronization.
//!
//! # Overview
//!
//! The e2e tests validate Madara's behavior in realistic scenarios by spawning actual node instances
//! and interacting with them through standard interfaces. Tests cover everything from basic RPC
//! functionality to complex multi-node setups with gateways and sequencers. The test infrastructure
//! handles node lifecycle management, port allocation, and provides utilities for asserting on
//! asynchronous blockchain state changes.
//!
//! The test suite is organized into specialized modules, each focusing on different aspects of
//! Madara's functionality. Tests can run against single nodes or complex multi-node topologies,
//! simulating production deployment patterns.
//!
//! # Test Infrastructure
//!
//! The core testing infrastructure revolves around two main components:
//!
//! - [`MadaraCmdBuilder`]: Constructs and configures Madara node instances
//! - [`MadaraCmd`]: Manages a running node instance and provides access to its APIs
//!
//! These components handle the complexity of spawning processes, capturing output, waiting for
//! nodes to be ready, and cleaning up resources after tests complete.
//!
//! # Node Configuration
//!
//! The [`MadaraCmdBuilder`] provides the main interface for configuring test nodes:
//!
//! ```no_run
//! let builder = MadaraCmdBuilder::new()
//!     .args(["--full", "--network", "sepolia"])
//!     .env([("MADARA_RPC_PORT", "9944")])
//!     .enable_gateway()
//!     .label("test_node");
//! ```
//!
//! Key configuration options include:
//! - **Network mode**: Full node, sequencer, or devnet.
//! - **Sync settings**: Block ranges, L1/L2 sync configuration.
//! - **API endpoints**: RPC and gateway enablement.
//! - **Database location**: Temporary directories managed automatically.
//!
//! # Running Test Nodes
//!
//! When a test node is started via [`MadaraCmdBuilder::run`]:
//!
//! 1. **Process spawning**: The Madara binary is launched with configured arguments.
//! 2. **Port allocation**: OS assigns available ports for RPC/gateway endpoints.
//! 3. **Output capture**: stdout/stderr are captured and parsed for port information.
//! 4. **Ready detection**: The test waits for the node to be fully operational.
//! 5. **API access**: JSON-RPC and gateway clients are configured automatically.
//!
//! # Test Modules
//!
//! ## RPC Tests (`rpc`)
//!
//! Validates all Starknet JSON-RPC methods against known responses from production networks.
//! Tests cover read methods, transaction submission, and trace generation. Each test includes
//! the exact curl command used to generate the expected response for reproducibility.
//!
//! ## Transaction Flow Tests (`transaction_flow`)
//!
//! Tests complex transaction scenarios including:
//!
//! - Normal transfers with proper nonce management.
//! - Concurrent transactions from multiple accounts.
//! - Deploy account transactions with wrong ordering.
//! - Contract declaration and deployment chains.
//! - Invalid transactions (duplicate, wrong nonce).
//!
//! ## Devnet Tests (`devnet`)
//!
//! Validates devnet-specific functionality:
//!
//! - Pre-funded account management.
//! - Custom chain configuration.
//! - Mempool persistence across restarts.
//! - Block production control.
//!
//! ## Storage Proof Tests (`storage_proof`)
//!
//! Tests Merkle proof generation for storage values, supporting both snapshot-based
//! and trie-log-based proof generation strategies.
//!
//! ## Gateway Tests (`gateway`)
//!
//! Validates the feeder gateway implementation, testing block retrieval and hash computation
//! across different Starknet versions.
//!
//! # Multi-Node Topologies
//!
//! The test infrastructure supports three deployment patterns via `TestSetup`:
//!
//! ## SequencerOnly
//!
//! A single node that handles both validation and execution. This is the simplest setup
//! for testing basic functionality.
//!
//! ## FullNodeAndSequencer
//!
//! Two nodes where:
//! - The sequencer validates and executes transactions
//! - The full node syncs from the sequencer's gateway and serves read requests
//!
//! ## GatewayAndSequencer
//!
//! Two nodes with separated concerns:
//! - The gateway node validates transactions and forwards valid ones to the sequencer
//! - The sequencer focuses solely on execution and block production
//!
//! # Utilities
//!
//! ## Waiting for Conditions
//!
//! The [`wait_for_cond`] utility enables testing asynchronous blockchain operations:
//!
//! ```no_run
//! wait_for_cond(
//!     || async {
//!         let receipt = node.json_rpc().get_transaction_receipt(tx_hash).await?;
//!         assert!(receipt.block.is_block());
//!         Ok(())
//!     },
//!     Duration::from_millis(500),  // poll interval
//!     60,                           // max attempts
//! ).await;
//! ```
//!
//! ## Test Accounts
//!
//! Devnet mode provides pre-funded accounts with known private keys:
//!
//! ```no_run
//! use mc_e2e_tests::devnet::{ACCOUNTS, ACCOUNT_SECRETS};
//!
//! let account_address = ACCOUNTS[0];
//! let private_key = ACCOUNT_SECRETS[0];
//! ```
//!
//! # Writing New Tests
//!
//! Tests typically follow this pattern:
//!
//! 1. **Setup**: Configure and start nodes using [`MadaraCmdBuilder`]
//! 2. **Wait for ready**: Ensure nodes are synchronized and APIs are available
//! 3. **Execute operations**: Submit transactions, make RPC calls, etc.
//! 4. **Assert results**: Use utilities like [`wait_for_cond`] for async assertions
//! 5. **Cleanup**: Automatic via [`Drop`] implementations
//!
//! ```no_run
//! #[tokio::test]
//! async fn test_transaction() {
//!     // Setup
//!     let mut node = MadaraCmdBuilder::new()
//!         .args(["--devnet"])
//!         .run();
//!     node.wait_for_ready().await;
//!
//!     // Execute
//!     let account = /* ... */;
//!     let tx_hash = account.execute_v3(/* ... */).send().await.unwrap();
//!
//!     // Assert
//!     wait_for_cond(/* check receipt */).await;
//!
//!     // Cleanup happens automatically
//! }
//! ```
//!
//! # Database Management
//!
//! Each test gets an isolated temporary database directory via [`TempDir`]. The directory
//! is automatically cleaned up when the test completes. For tests that need to restart
//! nodes with the same database, clone the [`MadaraCmdBuilder`] to reuse the same
//! temporary directory.
//!
//! # Environment Variables
//!
//! Tests respect several environment variables:
//!
//! - `COVERAGE_BIN`: Path to the Madara binary to test (required)
//! - `GATEWAY_KEY`: API key for gateway endpoints (optional)
//! - `RUST_LOG`: Controls logging verbosity
//!
//! [`MadaraCmdBuilder`]: crate::MadaraCmdBuilder
//! [`MadaraCmd`]: crate::MadaraCmd
//! [`wait_for_cond`]: crate::wait_for_cond
//! [`TempDir`]: tempfile::TempDir
//! [`Drop`]: std::ops::Drop
#![allow(clippy::print_stdout)]

#[cfg(test)]
mod devnet;
#[cfg(test)]
mod gateway;
#[cfg(test)]
mod rpc;
#[cfg(test)]
mod storage_proof;
#[cfg(test)]
mod transaction_flow;

use anyhow::bail;
use rstest::rstest;
use starknet_core::types::Felt;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Url};
use starknet_providers::{Provider, SequencerGatewayProvider};
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::mpsc::TryRecvError;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;
use std::{
    collections::HashMap,
    env,
    future::Future,
    path::{Path, PathBuf},
    process::{Child, Command, Output},
    time::Duration,
};
use tempfile::TempDir;

async fn wait_for_cond<F: Future<Output = Result<R, anyhow::Error>>, R>(
    mut cond: impl FnMut() -> F,
    sleep_duration: Duration,
    max_attempts: u32,
) -> R {
    let start = Instant::now();
    let mut attempt = 0;
    loop {
        let err = match cond().await {
            Ok(r) => break r,
            Err(err) => err,
        };

        attempt += 1;
        if attempt >= max_attempts {
            panic!("Condition not satisfied after {:?}: {err:#}", start.elapsed());
        }

        tokio::time::sleep(sleep_duration).await;
    }
}

pub struct MadaraCmd {
    process: Option<Child>,
    ready: bool,
    rpc_url: Option<Url>,
    gateway_root_url: Option<Url>,
    tempdir: Arc<TempDir>,
    label: String,
}

impl MadaraCmd {
    pub fn wait_with_output(mut self) -> Output {
        self.process.take().unwrap().wait_with_output().unwrap()
    }

    pub fn json_rpc(&self) -> JsonRpcClient<HttpTransport> {
        JsonRpcClient::new(HttpTransport::new(self.rpc_url.clone().unwrap()))
    }

    pub fn gateway_client(&self, chain_id: Felt) -> SequencerGatewayProvider {
        SequencerGatewayProvider::new(
            Url::parse(&self.gateway_url()).unwrap(),
            Url::parse(&self.feeder_gateway_url()).unwrap(),
            chain_id,
        )
    }

    pub async fn gateway_root_get(&self, endpoint: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().get(format!("{}{endpoint}", self.gateway_root_url.as_ref().unwrap()))
    }
    pub async fn gateway_root_post(&self, endpoint: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().post(format!("{}{endpoint}", self.gateway_root_url.as_ref().unwrap()))
    }

    pub fn rpc_url(&self) -> String {
        format!("{}", self.rpc_url.as_ref().unwrap())
    }
    pub fn gateway_url(&self) -> String {
        format!("{}/gateway", self.gateway_root_url.as_ref().unwrap())
    }
    pub fn feeder_gateway_url(&self) -> String {
        format!("{}/feeder_gateway", self.gateway_root_url.as_ref().unwrap())
    }

    pub fn db_dir(&self) -> &Path {
        self.tempdir.path()
    }

    pub async fn wait_for_ready(&mut self) -> &mut Self {
        let endpoint = self.rpc_url.as_ref().unwrap().join("/health").unwrap();
        wait_for_cond(
            || async {
                let res = reqwest::get(endpoint.clone()).await?;
                res.error_for_status()?;
                anyhow::Ok(())
            },
            Duration::from_millis(500),
            50,
        )
        .await;
        self.ready = true;
        self
    }

    // TODO: replace this with `subscribeNewHeads`
    pub async fn wait_for_sync_to(&mut self, block_n: u64) -> &mut Self {
        let rpc = self.json_rpc();
        wait_for_cond(
            || async {
                match rpc.block_hash_and_number().await {
                    Ok(got) => {
                        tracing::info!("Received block number {} out of {block_n}", got.block_number);

                        if got.block_number < block_n {
                            bail!("got block_n {}, expected {block_n}", got.block_number);
                        }
                        anyhow::Ok(())
                    }
                    Err(err) => bail!(err),
                }
            },
            Duration::from_secs(2),
            100,
        )
        .await;
        self
    }

    pub fn kill(&mut self) {
        let Some(mut child) = self.process.take() else { return };
        let _ = child.kill();
    }

    pub fn stop(&mut self) {
        let Some(mut child) = self.process.take() else { return };

        // Send SIGTERM signal to gracefully terminate the process
        let termination_result = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status();

        // Force kill if graceful termination failed
        if termination_result.is_err() {
            let _ = child.kill();
        }

        let grace_period = Duration::from_secs(5);
        let termination_start = std::time::Instant::now();

        // Wait for process exit or force kill after grace period
        while let Ok(None) = child.try_wait() {
            if termination_start.elapsed() >= grace_period {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Ensure process cleanup
        let _ = child.wait();
    }

    pub fn hook_stdout_and_wait_for_ports(&mut self, rpc: bool, gateway: bool) {
        let stderr =
            self.process.as_mut().unwrap().stderr.take().expect("Could not capture stderr from Madara process");
        let pid = self.process.as_ref().unwrap().id();

        let stdout_prefix = if !self.label.is_empty() { format!("[{pid} {}]", self.label) } else { format!("[{pid}]") };

        let reader = BufReader::new(stderr);
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let mut rpc_port = None;
            let mut gateway_port = None;

            for line in reader.lines().map_while(Result::ok) {
                fn get_port(line: &str, prefix: &str) -> Option<u16> {
                    if let Some(addr_part) = line.split(prefix).nth(1) {
                        if let Some(ip_port) = addr_part.split_whitespace().next() {
                            if let Some(port_str) = ip_port.rsplit(':').next() {
                                if let Ok(port) = port_str.parse::<u16>() {
                                    return Some(port);
                                }
                            }
                        }
                    }
                    None
                }

                rpc_port = rpc_port.or(get_port(&line, "Running JSON-RPC server at "));
                gateway_port = gateway_port.or(get_port(&line, "Gateway endpoint started at "));

                if (!rpc && rpc_port.is_some()) || (!gateway && gateway_port.is_some()) {
                    panic!(
                        "Inconsistent returned ports: expected rpc_enabled={rpc}, gateway_enabled={gateway}, \
                        got rpc_port={rpc_port:?}, gateway_port={gateway_port:?}"
                    )
                }

                if (rpc == rpc_port.is_some()) && (gateway == gateway_port.is_some()) {
                    let _ = tx.send((rpc_port, gateway_port));
                }
                println!("{stdout_prefix} {line}");
            }
        });

        let timeout = Duration::from_secs(30);
        let start = Instant::now();

        while start.elapsed() < timeout {
            match rx.try_recv() {
                Ok((rpc_port, gateway_port)) => {
                    let rpc_url = rpc_port.map(|port| Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap());
                    let gateway_root_url =
                        gateway_port.map(|port| Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap());

                    self.rpc_url = rpc_url;
                    self.gateway_root_url = gateway_root_url;
                    return;
                }
                Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => {
                    panic!("Port extraction thread terminated unexpectedly")
                }
            }
        }

        panic!("Timed out after {timeout:?} waiting for Madara to start")
    }
}

impl Drop for MadaraCmd {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Note: the builder is [`Clone`]able. When cloned, it will keep the same tempdir.
///
/// This is useful for tests that need to restart the node using the same DB: they
/// can just make a builder, clone() it and call [`MadaraCmdBuilder::run`] to launch
/// the node. They can then [`drop`] the [`MadaraCmd`] instance to kill the node, and
/// restart the node using the same db by reusing the earlier builder.
#[derive(Clone)]
pub struct MadaraCmdBuilder {
    args: Vec<String>,
    env: HashMap<String, String>,
    tempdir: Arc<TempDir>,
    rpc_enabled: bool,
    gateway_enabled: bool,
    label: String,
}

impl Default for MadaraCmdBuilder {
    fn default() -> Self {
        Self {
            args: Default::default(),
            env: Default::default(),
            tempdir: Arc::new(TempDir::with_prefix("madara-test").unwrap()),
            rpc_enabled: true,
            gateway_enabled: false,
            label: String::new(),
        }
    }
}

impl MadaraCmdBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn no_rpc(self) -> Self {
        Self { rpc_enabled: false, ..self }
    }
    pub fn enable_gateway(self) -> Self {
        Self { gateway_enabled: true, ..self }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn env(mut self, env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        self.env = env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Also waits for the ports to be assigned.
    pub fn run(self) -> MadaraCmd {
        let (rpc, gateway) = (self.rpc_enabled, self.gateway_enabled);
        let mut cmd = self.run_no_wait();
        cmd.hook_stdout_and_wait_for_ports(rpc, gateway);
        cmd
    }

    pub fn run_no_wait(self) -> MadaraCmd {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let target_bin = PathBuf::from(env::var("COVERAGE_BIN").expect("env COVERAGE_BIN to be set by script"));

        assert!(target_bin.exists(), "No binary to run: {:?}", target_bin);

        let gateway_key_args =
            env::var("GATEWAY_KEY").ok().map(|key| vec!["--gateway-key".into(), key]).unwrap_or_default();

        tracing::info!("Running new madara process with args {:?}", self.args);

        let mut cmd = Command::new(target_bin);
        cmd.envs(self.env)
            .args(self.args)
            .args(["--base-path".into(), self.tempdir.path().display().to_string()])
            .args(
                self.rpc_enabled
                    .then_some([
                        "--rpc-port",
                        "0", // OS Assigned
                    ])
                    .into_iter()
                    .flatten(),
            )
            .args(
                self.gateway_enabled
                    .then_some([
                        "--gateway-port",
                        "0", // OS Assigned
                    ])
                    .into_iter()
                    .flatten(),
            )
            .args(gateway_key_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let process = cmd.spawn().expect("Failed to spawn Madara process");

        MadaraCmd {
            process: Some(process),
            ready: false,
            rpc_url: None,
            gateway_root_url: None,
            label: self.label,
            tempdir: self.tempdir,
        }
    }
}

#[rstest]
fn madara_help_shows() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let output = MadaraCmdBuilder::new().args(["--help"]).run_no_wait().wait_with_output();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Madara: High performance Starknet sequencer/full-node"), "stdout: {stdout}");
}

#[rstest]
#[tokio::test]
async fn madara_can_sync_a_few_blocks() {
    use starknet_core::types::BlockHashAndNumber;
    use starknet_types_core::felt::Felt;

    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "--network",
        "sepolia",
        "--sync-stop-at",
        "19",
        "--no-l1-sync",
        "--l1-gas-price",
        "0",
    ]);

    let mut node = cmd_builder.run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(19).await;

    assert_eq!(
        node.json_rpc().block_hash_and_number().await.unwrap(),
        BlockHashAndNumber {
            // https://sepolia.voyager.online/block/19
            block_hash: Felt::from_hex_unchecked("0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5"),
            block_number: 19
        }
    );
}

#[rstest]
#[tokio::test]
async fn madara_can_sync_and_restart() {
    use starknet_core::types::BlockHashAndNumber;
    use starknet_types_core::felt::Felt;

    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--full",
        "--network",
        "sepolia",
        "--sync-stop-at",
        "5",
        "--no-l1-sync",
        "--l1-gas-price",
        "0",
    ]);

    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(5).await;

    assert_eq!(
        node.json_rpc().block_hash_and_number().await.unwrap(),
        BlockHashAndNumber {
            // https://sepolia.voyager.online/block/5
            block_hash: Felt::from_hex_unchecked("0x13b390a0b2c48f907cda28c73a12aa31b96d51bc1be004ba5f71174d8d70e4f"),
            block_number: 5
        }
    );

    node.stop(); // stop the node (gracefully).

    let cmd_builder = cmd_builder.args([
        "--full",
        "--network",
        "sepolia",
        "--sync-stop-at",
        "7",
        "--no-l1-sync",
        "--l1-gas-price",
        "0",
    ]);

    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(7).await;

    assert_eq!(
        node.json_rpc().block_hash_and_number().await.unwrap(),
        BlockHashAndNumber {
            // https://sepolia.voyager.online/block/7
            block_hash: Felt::from_hex_unchecked("0x2e59a5adbdf53e00fd282a007b59771067870c1c7664ca7878327adfff398b4"),
            block_number: 7
        }
    );

    node.kill(); // kill the node. ungraceful shutdown.

    let cmd_builder = cmd_builder.args([
        "--full",
        "--network",
        "sepolia",
        "--sync-stop-at",
        "10",
        "--no-l1-sync",
        "--l1-gas-price",
        "0",
    ]);

    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(10).await;

    assert_eq!(
        node.json_rpc().block_hash_and_number().await.unwrap(),
        BlockHashAndNumber {
            // https://sepolia.voyager.online/block/10
            block_hash: Felt::from_hex_unchecked("0x3b26e3fc6bc2062f99479ea06a79e080a5f373514e03002459010c3be544593"),
            block_number: 10
        }
    );
}
