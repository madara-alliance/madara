use std::env;
use std::ops::Range;
use std::sync::Mutex;
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tempfile::TempDir;
use url::Url;

// This code has been take from [here](https://github.com/madara-alliance/madara/blob/main/crates/tests/src/lib.rs)
// and modified to fit the needs of this project.
pub struct MadaraCmd {
    pub process: Option<Child>,
    pub ready: bool,
    pub rpc_url: Url,
    pub tempdir: TempDir,
    pub _port: MadaraPortNum,
}

pub async fn wait_for_cond<F: Future<Output = Result<bool, anyhow::Error>>>(
    mut cond: impl FnMut() -> F,
    duration: Duration,
) -> Result<bool, anyhow::Error> {
    let mut attempt = 0;
    loop {
        let err = match cond().await {
            Ok(result) => return Ok(result),
            Err(err) => {
                // Empty block, no action needed
                err
            }
        };

        attempt += 1;
        if attempt >= 10 {
            panic!("No answer from the node after {attempt} attempts: {:#}", err)
        }

        tokio::time::sleep(duration).await;
    }
}

impl MadaraCmd {
    pub fn db_dir(&self) -> &Path {
        self.tempdir.path()
    }

    pub async fn wait_for_ready(&mut self) -> &mut Self {
        let endpoint = self.rpc_url.join("/health").unwrap();
        wait_for_cond(
            || async {
                let res = reqwest::get(endpoint.clone()).await?;
                res.error_for_status()?;
                anyhow::Ok(true)
            },
            Duration::from_millis(1000),
        )
        .await
        .expect("Could not get health of Madara");
        self.ready = true;
        self
    }
}

impl Drop for MadaraCmd {
    fn drop(&mut self) {
        let Some(mut child) = self.process.take() else { return };
        let kill = || {
            let mut kill = Command::new("kill").args(["-s", "TERM", &child.id().to_string()]).spawn()?;
            kill.wait()?;
            anyhow::Ok(())
        };
        if let Err(_err) = kill() {
            child.kill().unwrap()
        }
        child.wait().unwrap();
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
    MadaraPortNum(guard.next.next().expect("no more port to use"))
}

pub struct MadaraCmdBuilder {
    args: Vec<String>,
    env: HashMap<String, String>,
    tempdir: TempDir,
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
            tempdir: TempDir::with_prefix("madara-test").unwrap(),
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
        let target_bin = env::var("MADARA_BINARY_PATH").expect("failed to get binary path");
        let target_bin = PathBuf::from(target_bin);

        if !target_bin.exists() {
            panic!("No binary to run: {:?}", target_bin)
        }

        let process = Command::new(target_bin)
            .envs(self.env)
            .args(self.args.into_iter().chain([
                "--telemetry-disabled".into(), // important: disable telemetry!!
                "--no-prometheus".into(),
                "--base-path".into(),
                format!("{}", self.tempdir.as_ref().display()),
                "--rpc-port".into(),
                format!("{}", self.port.0),
            ]))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();

        MadaraCmd {
            process: Some(process),
            ready: false,
            rpc_url: Url::parse(&format!("http://127.0.0.1:{}/", self.port.0)).unwrap(),
            tempdir: self.tempdir,
            _port: self.port,
        }
    }
}
