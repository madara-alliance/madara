//! End to end tests for madara.
#![cfg(test)]

mod devnet;
mod rpc;
mod storage_proof;

use anyhow::bail;
use rstest::rstest;
use starknet_providers::Provider;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Url};
use std::{
    collections::HashMap,
    env,
    future::Future,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        mpsc::{self, TryRecvError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;

async fn wait_for_cond<F: Future<Output = Result<(), anyhow::Error>>>(
    mut cond: impl FnMut() -> F,
    duration: Duration,
    max_attempts: u32,
) {
    let mut attempt = 0;
    loop {
        let Err(err) = cond().await else {
            break;
        };

        attempt += 1;
        if attempt >= max_attempts {
            let elapsed = (attempt as f64 * duration.as_secs() as f64) / 60.0;
            panic!("No answer from the node after {:.1} minutes: {:#}", elapsed, err);
        }

        tokio::time::sleep(duration).await;
    }
}

pub struct MadaraCmd {
    process: Option<Child>,
    ready: bool,
    json_rpc: JsonRpcClient<HttpTransport>,
    rpc_url: Url,
    tempdir: Arc<TempDir>,
    _port: u16,
}

impl MadaraCmd {
    pub fn wait_with_output(mut self) -> Output {
        self.process.take().unwrap().wait_with_output().unwrap()
    }

    pub fn json_rpc(&self) -> &JsonRpcClient<HttpTransport> {
        &self.json_rpc
    }

    pub fn db_dir(&self) -> &Path {
        self.tempdir.path()
    }

    pub async fn wait_for_ready(&mut self) -> &mut Self {
        let endpoint = self.rpc_url.join("/health").unwrap();
        wait_for_cond(
            || async {
                let res = reqwest::get(endpoint.clone()).await?;
                res.error_for_status()?;
                anyhow::Ok(())
            },
            Duration::from_millis(500),
            100,
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
            400,
        )
        .await;
        self
    }
}

impl Drop for MadaraCmd {
    fn drop(&mut self) {
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
}

pub fn extract_port_from_stderr(process: &mut Child) -> Result<u16, String> {
    let stderr = process.stderr.take().ok_or_else(|| "Could not capture stderr from Madara process".to_string())?;

    let reader = BufReader::new(stderr);
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        for line in reader.lines().map_while(Result::ok) {
            if let Some(addr_part) = line.split("Running JSON-RPC server at ").nth(1) {
                if let Some(ip_port) = addr_part.split_whitespace().next() {
                    if let Some(port_str) = ip_port.rsplit(':').next() {
                        if let Ok(port) = port_str.parse::<u16>() {
                            let _ = tx.send(port);
                            return;
                        }
                    }
                }
            }
        }
    });

    let timeout = Duration::from_secs(30);
    let start = Instant::now();

    while start.elapsed() < timeout {
        match rx.try_recv() {
            Ok(port) => return Ok(port),
            Err(TryRecvError::Empty) => thread::sleep(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => return Err("Port extraction thread terminated unexpectedly".to_string()),
        }
    }

    Err(format!("Timed out after {:?} waiting for Madara to start", timeout))
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
}

impl Default for MadaraCmdBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MadaraCmdBuilder {
    pub fn new() -> Self {
        Self {
            args: Default::default(),
            env: Default::default(),
            tempdir: Arc::new(TempDir::with_prefix("madara-test").unwrap()),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn env(mut self, env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        self.env = env.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        self
    }

    pub fn run(self) -> MadaraCmd {
        self.run_with_option(true)
    }

    pub fn run_with_option(self, wait_for_port: bool) -> MadaraCmd {
        let target_bin = PathBuf::from(env::var("COVERAGE_BIN").expect("env COVERAGE_BIN to be set by script"));

        assert!(target_bin.exists(), "No binary to run: {:?}", target_bin);

        let gateway_key_args =
            env::var("GATEWAY_KEY").ok().map(|key| vec!["--gateway-key".into(), key]).unwrap_or_default();

        let mut cmd = Command::new(target_bin);
        cmd.envs(self.env)
            .args(self.args)
            .args([
                "--base-path".into(),
                self.tempdir.path().display().to_string(),
                "--rpc-port".into(),
                "0".into(), // OS Assigned
            ])
            .args(gateway_key_args)
            .stdout(if wait_for_port { Stdio::inherit() } else { Stdio::piped() })
            .stderr(Stdio::piped());

        let mut process = cmd.spawn().expect("Failed to spawn Madara process");

        let port = if wait_for_port {
            let actual_port =
                extract_port_from_stderr(&mut process).expect("Failed to extract port from Madara process");
            tracing::info!("Detected Madara running on port: {}", actual_port);
            actual_port
        } else {
            0
        };

        let rpc_url = Url::parse(&format!("http://127.0.0.1:{}/", port)).unwrap();

        MadaraCmd {
            process: Some(process),
            ready: false,
            json_rpc: JsonRpcClient::new(HttpTransport::new(rpc_url.clone())),
            rpc_url,
            tempdir: self.tempdir,
            _port: port,
        }
    }
}

#[rstest]
fn madara_help_shows() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let output = MadaraCmdBuilder::new().args(["--help"]).run_with_option(false).wait_with_output();
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
        "--no-sync-polling",
        "--n-blocks-to-sync",
        "20",
        "--no-l1-sync",
        "--gas-price",
        "0",
    ]);

    let mut node = cmd_builder.run();
    node.wait_for_ready().await;
    node.wait_for_sync_to(19).await;

    assert_eq!(
        node.json_rpc().block_hash_and_number().await.unwrap(),
        BlockHashAndNumber {
            // https://sepolia.voyager.online/block/0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5
            block_hash: Felt::from_hex_unchecked("0x4177d1ba942a4ab94f86a476c06f0f9e02363ad410cdf177c54064788c9bcb5"),
            block_number: 19
        }
    );
}
