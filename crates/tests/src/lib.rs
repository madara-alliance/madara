//! End to end tests for madara.
#![cfg(test)]

mod rpc;
mod storage_proof;

use anyhow::bail;
use rstest::rstest;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::signers::{LocalWallet, SigningKey};
use starknet_core::types::{BlockId, BlockTag, Call, Felt};
use starknet_core::utils::starknet_keccak;
use starknet_providers::Provider;
use starknet_providers::{jsonrpc::HttpTransport, JsonRpcClient, Url};
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
            panic!("No answer from the node after {attempt} attempts: {:#}", err)
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
    _port: MadaraPortNum,
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
            20,
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

#[derive(Clone)]
pub struct MadaraCmdBuilder {
    args: Vec<String>,
    env: HashMap<String, String>,
    tempdir: Arc<TempDir>,
    port: MadaraPortNum,
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
            port: get_port(),
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
        let target_bin = env::var("COVERAGE_BIN").expect("env COVERAGE_BIN to be set by script");
        let target_bin = PathBuf::from_str(&target_bin).expect("COVERAGE_BIN to be a path");
        if !target_bin.exists() {
            panic!("No binary to run: {:?}", target_bin)
        }

        // This is an optional argument to sync faster from the FGW if gateway_key is set
        let gateway_key_arg = env::var("GATEWAY_KEY").ok().map(|gateway_key| ["--gateway-key".into(), gateway_key]);

        let process = Command::new(target_bin)
            .envs(self.env)
            .args(
                self.args
                    .into_iter()
                    .chain([
                        "--base-path".into(),
                        format!("{}", self.tempdir.deref().as_ref().display()),
                        "--rpc-port".into(),
                        format!("{}", self.port.0),
                    ])
                    .chain(gateway_key_arg.into_iter().flatten()),
            )
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        let rpc_url = Url::parse(&format!("http://127.0.0.1:{}/", self.port.0)).unwrap();
        MadaraCmd {
            process: Some(process),
            ready: false,
            json_rpc: JsonRpcClient::new(HttpTransport::new(rpc_url.clone())),
            rpc_url,
            tempdir: self.tempdir,
            _port: self.port,
        }
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

const ERC20_STRK_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");
#[allow(unused)]
const ERC20_ETH_CONTRACT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7");

const ACCOUNT_SECRET: Felt =
    Felt::from_hex_unchecked("0x077e56c6dc32d40a67f6f7e6625c8dc5e570abe49c0a24e9202e4ae906abcc07");
const ACCOUNT_ADDRESS: Felt =
    Felt::from_hex_unchecked("0x055be462e718c4166d656d11f89e341115b8bc82389c3762a10eade04fcb225d");

#[rstest]
#[tokio::test]
async fn madara_devnet_add_transaction() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let args = &[
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // only produce blocks no pending txs
        "--chain-config-override",
        "block_time=1s,pending_block_update_time=1s",
    ];

    let cmd_builder = MadaraCmdBuilder::new().args(*args);
    let mut node = cmd_builder.run();
    node.wait_for_ready().await;

    tokio::time::sleep(Duration::from_secs(3)).await;

    let chain_id = node.json_rpc().chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Latest));

    let res = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 15.into(), Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();

    wait_for_cond(
        || async {
            let _receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;
}

#[rstest]
#[tokio::test]
async fn madara_devnet_mempool_saving() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // never produce blocks & pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=5min",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    let chain_id = node.json_rpc().chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    let res = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 15.into(), Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();

    drop(node);

    // tx should be in saved mempool

    let cmd_builder = cmd_builder.args([
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // never produce blocks but produce pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=500ms",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    // tx should be in mempool

    wait_for_cond(
        || async {
            let _receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;
}

#[rstest]
#[tokio::test]
async fn madara_devnet_continue_pending() {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let cmd_builder = MadaraCmdBuilder::new().args([
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // never produce blocks but produce pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=500ms",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    let chain_id = node.json_rpc().chain_id().await.unwrap();

    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(ACCOUNT_SECRET));
    let mut account =
        SingleOwnerAccount::new(node.json_rpc(), signer, ACCOUNT_ADDRESS, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::Pending));

    let res = account
        .execute_v3(vec![Call {
            to: ERC20_STRK_CONTRACT_ADDRESS,
            selector: starknet_keccak(b"transfer"),
            calldata: vec![ACCOUNT_ADDRESS, 15.into(), Felt::ZERO],
        }])
        .send()
        .await
        .unwrap();

    wait_for_cond(
        || async {
            let _receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await?;
            Ok(())
        },
        Duration::from_millis(500),
        60,
    )
    .await;

    drop(node);

    // tx should appear in saved pending block

    let cmd_builder = cmd_builder.args([
        "--devnet",
        "--no-l1-sync",
        "--gas-price",
        "0",
        // never produce blocks never produce pending txs
        "--chain-config-path",
        "test_devnet.yaml",
        "--chain-config-override",
        "block_time=5min,pending_block_update_time=5min",
    ]);
    let mut node = cmd_builder.clone().run();
    node.wait_for_ready().await;

    // should find receipt
    let _receipt = node.json_rpc().get_transaction_receipt(res.transaction_hash).await.unwrap();
}
