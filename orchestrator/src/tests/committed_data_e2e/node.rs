//! Owned Madara processes for the committed-data acceptance test. Every database and port is
//! isolated; dropping a handle kills and reaps only the child created by this test.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

pub(super) struct Node {
    child: Child,
    args: Vec<String>,
    binary: PathBuf,
    pub rpc: String,
    pub admin: String,
    pub private: String,
    pub feeder: String,
    pub directory: tempfile::TempDir,
}

pub(super) async fn rpc(url: &str, method: &str, params: Value) -> Result<Value> {
    let response: Value = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_secs(120))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(response.get("error").is_none(), "{method}: {response}");
    response.get("result").cloned().context("RPC result missing")
}

impl Node {
    pub async fn start(upstream: Option<&Self>) -> Result<Self> {
        Self::start_with_data(upstream, true).await
    }

    /// Feature-disabled reader exercises the unchanged block/bouncer protocol. This emulates
    /// the legacy sync path; it is not evidence about an untested older executable.
    pub async fn start_with_data(upstream: Option<&Self>, committed_data: bool) -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let binary = PathBuf::from(
            std::env::var("MADARA_ORCHESTRATOR_MADARA_BINARY_PATH")
                .context("acceptance test requires the CI-built Madara binary")?,
        );
        // Hold all four reservations until spawn so the OS cannot reuse a port within this node.
        let reservations = (0..4).map(|_| TcpListener::bind("127.0.0.1:0")).collect::<std::io::Result<Vec<_>>>()?;
        let rpc_port = reservations[0].local_addr()?.port();
        let admin_port = reservations[1].local_addr()?.port();
        let private_port = reservations[2].local_addr()?.port();
        let gateway_port = reservations[3].local_addr()?.port();
        let mut args: Vec<String> = [
            "--base-path",
            directory.path().to_str().unwrap(),
            "--l1-sync-disabled",
            "--otel-export-metrics",
            "false",
            "--rpc-admin",
            "--rpc-unsafe",
            "--feeder-gateway-enable",
            "--gateway-enable",
            "--rpc-port",
            &rpc_port.to_string(),
            "--rpc-admin-port",
            &admin_port.to_string(),
            "--gateway-port",
            &gateway_port.to_string(),
            "--committed-data-memory-blocks",
            "0",
            "--committed-data-cache-mib",
            "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if committed_data {
            args.push("--use-committed-data".into());
        }
        match upstream {
            Some(upstream) => args.extend([
                "--full".into(),
                "--preset".into(),
                "devnet".into(),
                "--gateway-url".into(),
                upstream.feeder.trim_end_matches("/feeder_gateway/").into(),
                "--madara-additional-data-sync".into(),
            ]),
            None => args.extend([
                "--devnet".into(),
                "--rpc-committed-data".into(),
                "--rpc-committed-data-port".into(),
                private_port.to_string(),
                "--external-db-enabled".into(),
                "--external-db-mongodb-uri".into(),
                "mongodb://127.0.0.1:27017".into(),
                "--external-db-database".into(),
                format!("committed_e2e_{rpc_port}"),
                "--chain-config-override".into(),
                "block_time=600s,pending_block_update_time=100ms".into(),
                "--l1-gas-price".into(),
                "1000000000".into(),
                "--blob-gas-price".into(),
                "1".into(),
                "--strk-per-eth".into(),
                "1000".into(),
            ]),
        }
        drop(reservations);
        let child = Self::spawn(&binary, &args, directory.path())?;
        let mut node = Self {
            child,
            args,
            binary,
            rpc: format!("http://127.0.0.1:{rpc_port}/rpc/v0_10_2/"),
            admin: format!("http://127.0.0.1:{admin_port}/rpc/v0_1_0/"),
            private: format!("http://127.0.0.1:{private_port}/rpc/v0_1_0/"),
            feeder: format!("http://127.0.0.1:{gateway_port}/feeder_gateway/"),
            directory,
        };
        node.ready().await?;
        Ok(node)
    }

    fn spawn(binary: &PathBuf, args: &[String], directory: &std::path::Path) -> Result<Child> {
        let log = std::fs::OpenOptions::new().create(true).append(true).open(directory.join("madara.log"))?;
        Ok(Command::new(binary)
            .args(args)
            .env("RUST_LOG", "info")
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()?)
    }

    async fn ready(&mut self) -> Result<()> {
        for _ in 0..120 {
            ensure!(
                self.child.try_wait()?.is_none(),
                "Madara exited: {}",
                std::fs::read_to_string(self.directory.path().join("madara.log"))?
            );
            if rpc(&self.rpc, "starknet_chainId", json!([])).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        anyhow::bail!("Madara startup deadline exceeded")
    }

    pub async fn receipt(&self, hash: starknet_types_core::felt::Felt) -> Result<Value> {
        for _ in 0..120 {
            if let Ok(receipt) = rpc(&self.rpc, "starknet_getTransactionReceipt", json!([hash])).await {
                if receipt.get("block_hash").is_some_and(|hash| !hash.is_null()) {
                    return Ok(receipt);
                }
            }
            rpc(&self.admin, "madara_closeBlock", json!([])).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        anyhow::bail!("transaction confirmation deadline exceeded")
    }

    /// Sample the child process, not the test runner (which also constructs reference trees and
    /// executes SNOS). On Linux VmHWM is peak RSS since this child started; a restart resets it.
    /// Disk bytes include every file currently in the isolated directory, including WAL/logs.
    pub fn footprint(&self) -> Result<Value> {
        fn directory_bytes(path: &std::path::Path) -> std::io::Result<u64> {
            let mut bytes = 0;
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    // Compaction may unlink an obsolete SST between read_dir and stat.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                bytes += if metadata.is_dir() { directory_bytes(&entry.path())? } else { metadata.len() };
            }
            Ok(bytes)
        }
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.child.id())).ok();
        let kib = |name: &str| {
            status.as_ref().and_then(|status| {
                status.lines().find_map(|line| {
                    let mut fields = line.split_whitespace();
                    (fields.next()? == name).then(|| fields.next()?.parse::<u64>().ok()).flatten()
                })
            })
        };
        Ok(json!({"directory_bytes_including_wal_and_logs": directory_bytes(self.directory.path())?,
            "rss_kib": kib("VmRSS:"), "peak_rss_kib": kib("VmHWM:")}))
    }

    pub async fn restart_after_revert(&mut self, block_hash: Value) -> Result<()> {
        // The admin call stops services, rolls back chain state, then exits. A connection close
        // can race its JSON reply, so the postcondition is checked after waiting and reopening.
        let _reply = rpc(&self.admin, "madara_revertToAndShutdown", json!([block_hash])).await;
        for _ in 0..120 {
            if self.child.try_wait()?.is_some() {
                self.child = Self::spawn(&self.binary, &self.args, self.directory.path())?;
                return self.ready().await;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        anyhow::bail!("revert did not stop Madara")
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Ok(output) = std::env::var("COMMITTED_DATA_E2E_OUTPUT") {
            let _ = std::fs::create_dir_all(&output);
            let _ = std::fs::copy(
                self.directory.path().join("madara.log"),
                PathBuf::from(output).join(format!("node-{}.log", self.child.id())),
            );
        }
    }
}
