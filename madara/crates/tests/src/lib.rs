//! End to end tests for madara.
#![cfg(test)]

mod devnet;
mod rpc;
mod sequencing;
mod storage_proof;

use anyhow::bail;
use rstest::rstest;
use starknet_core::types::Felt;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Url};
use starknet_providers::{Provider, SequencerGatewayProvider};
use std::ops::{Deref, Range};
use std::sync::{Arc, Mutex};
use std::{
    collections::HashMap,
    env,
    future::Future,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    str::FromStr,
    time::Duration,
};
use tempfile::TempDir;

async fn wait_for_cond<F: Future<Output = Result<R, anyhow::Error>>, R>(
    mut cond: impl FnMut() -> F,
    duration: Duration,
    max_attempts: u32,
) -> R {
    let mut attempt = 0;
    loop {
        let err = match cond().await {
            Ok(r) => break r,
            Err(err) => err,
        };

        attempt += 1;
        if attempt >= max_attempts {
            panic!("No answer from the node after {attempt} attempts: {:#}", err)
        }

        tokio::time::sleep(duration).await;
    }
}

pub struct MadaraCmd {
    process: Option<Child>,
    ready: bool,
    json_rpc: Option<JsonRpcClient<HttpTransport>>,
    rpc_url: Option<Url>,
    gateway_root_url: Option<Url>,
    tempdir: Arc<TempDir>,
    _rpc_port: Option<MadaraPortNum>,
    _gateway_port: Option<MadaraPortNum>,
}

impl MadaraCmd {
    pub fn wait_with_output(mut self) -> Output {
        self.process.take().unwrap().wait_with_output().unwrap()
    }

    pub fn json_rpc(&self) -> &JsonRpcClient<HttpTransport> {
        self.json_rpc.as_ref().unwrap()
    }

    pub fn gateway_client(&self, chain_id: Felt) -> SequencerGatewayProvider {
        SequencerGatewayProvider::new(
            Url::parse(&format!("{}/gateway", self.gateway_root_url.as_ref().unwrap())).unwrap(),
            Url::parse(&format!("{}/feeder_gateway", self.gateway_root_url.as_ref().unwrap())).unwrap(),
            chain_id,
        )
    }

    pub async fn gateway_root_get(&self, endpoint: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().get(format!("{}{endpoint}", self.gateway_root_url.as_ref().unwrap()))
    }
    pub async fn gateway_root_post(&self, endpoint: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new().post(format!("{}{endpoint}", self.gateway_root_url.as_ref().unwrap()))
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

        let grace_period = Duration::from_secs(2);
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

// this really should use unix sockets, sad

const PORT_RANGE: Range<u16> = 19944..20000;

struct AvailablePorts<I: Iterator<Item = u16>> {
    to_reuse: Vec<u16>,
    next: I,
}

lazy_static::lazy_static! {
    static ref AVAILABLE_PORTS: Mutex<AvailablePorts<Range<u16>>> = Mutex::new(AvailablePorts { to_reuse: vec![], next: PORT_RANGE });
}

#[derive(Clone)]
pub struct MadaraPortNum(pub u16);
impl Drop for MadaraPortNum {
    fn drop(&mut self) {
        let mut guard = AVAILABLE_PORTS.lock().expect("poisoned lock");
        guard.to_reuse.push(self.0);
    }
}

pub fn get_port() -> MadaraPortNum {
    let mut guard = AVAILABLE_PORTS.lock().expect("poisoned lock");
    if let Some(el) = guard.to_reuse.pop() {
        return MadaraPortNum(el);
    }
    let port = guard.next.next().expect("no more port to use");
    MadaraPortNum(port)
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
    rpc_port: Option<MadaraPortNum>,
    gateway_port: Option<MadaraPortNum>,
    label: String,
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
            rpc_port: Some(get_port()),
            gateway_port: None,
            label: String::new(), // no label
        }
    }

    pub fn no_rpc(self) -> Self {
        Self { rpc_port: None, ..self }
    }
    pub fn enable_gateway(self) -> Self {
        Self { gateway_port: Some(get_port()), ..self }
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

    pub fn run(self) -> MadaraCmd {
        let target_bin = env::var("COVERAGE_BIN").expect("env COVERAGE_BIN to be set by script");
        let target_bin = PathBuf::from_str(&target_bin).expect("COVERAGE_BIN to be a path");
        if !target_bin.exists() {
            panic!("No binary to run: {:?}", target_bin)
        }

        // This is an optional argument to sync faster from the FGW if gateway_key is set
        let gateway_key_arg = env::var("GATEWAY_KEY").ok().map(|gateway_key| ["--gateway-key".into(), gateway_key]);

        let process = Command::new(target_bin)
            .envs(self.env.into_iter().chain([("MADARA_LOG_CUSTOM_PROCESS_LABEL".into(), self.label)])) // also apply the label, for nicer output formatting. Empty string will only show PID
            .args(
                self.args
                    .into_iter()
                    .chain(["--base-path".into(), format!("{}", self.tempdir.deref().as_ref().display())])
                    .chain(
                        self.rpc_port.as_ref().map(|p| ["--rpc-port".into(), format!("{}", p.0)]).into_iter().flatten(),
                    )
                    .chain(
                        self.gateway_port
                            .as_ref()
                            .map(|p| ["--gateway-port".into(), format!("{}", p.0)])
                            .into_iter()
                            .flatten(),
                    )
                    .chain(gateway_key_arg.into_iter().flatten()),
            )
            // .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let rpc_url = self.rpc_port.as_ref().map(|port| Url::parse(&format!("http://127.0.0.1:{}/", port.0)).unwrap());
        let gateway_root_url =
            self.gateway_port.as_ref().map(|port| Url::parse(&format!("http://127.0.0.1:{}/", port.0)).unwrap());
        MadaraCmd {
            process: Some(process),
            ready: false,
            json_rpc: rpc_url.as_ref().map(|rpc_url| JsonRpcClient::new(HttpTransport::new(rpc_url.clone()))),
            rpc_url,
            gateway_root_url,
            tempdir: self.tempdir,
            _rpc_port: self.rpc_port,
            _gateway_port: self.gateway_port,
        }
    }

    fn gateway_root_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.gateway_port.as_ref().unwrap().0)
    }
    fn gateway_url(&self) -> String {
        format!("{}/gateway", self.gateway_root_url())
    }
    fn feeder_gateway_url(&self) -> String {
        format!("{}/feeder_gateway", self.gateway_root_url())
    }
}

#[rstest]
fn madara_help_shows() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let output = MadaraCmdBuilder::new().args(["--help"]).run().wait_with_output();
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
