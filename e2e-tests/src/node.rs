use std::fs::{create_dir_all, File};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use tokio::net::TcpStream;
use url::Url;

use crate::{get_free_port, get_repository_root};

const CONNECTION_ATTEMPTS: usize = 360;
const CONNECTION_ATTEMPT_DELAY_MS: u64 = 500;

#[derive(Debug)]
pub struct Orchestrator {
    process: Child,
    address: String,
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        let mut kill =
            Command::new("kill").args(["-s", "TERM", &self.process.id().to_string()]).spawn().expect("Failed to kill");
        kill.wait().expect("Failed to kill the process");
    }
}

impl Orchestrator {
    fn cargo_run(root_dir: &Path, binary: &str, args: Vec<&str>, envs: Vec<(&str, &str)>) -> Child {
        let arguments = [vec!["run", "--bin", binary, "--release", "--"], args].concat();

        let logs_dir = Path::join(root_dir, Path::new("target/logs"));
        create_dir_all(logs_dir.clone()).expect("Failed to create logs dir");

        let stdout = Stdio::from(File::create(logs_dir.join(format!("{}-stdout.txt", binary))).unwrap());
        let stderr = Stdio::from(File::create(logs_dir.join(format!("{}-stderr.txt", binary))).unwrap());

        Command::new("cargo")
            .stdout(stdout)
            .stderr(stderr)
            .envs(envs)
            .args(arguments)
            .spawn()
            .expect("Could not run orchestrator node")
    }

    pub fn run(envs: Vec<(&str, &str)>) -> Self {
        let port = get_free_port();
        let address = format!("127.0.0.1:{}", port);
        let repository_root = &get_repository_root();

        std::env::set_current_dir(repository_root).expect("Failed to change working directory");

        let port_str = format!("{}", port);
        let envs = [envs, vec![("PORT", port_str.as_str())]].concat();

        let process = Self::cargo_run(repository_root.as_path(), "orchestrator", vec![], envs);

        Self { process, address }
    }

    pub fn endpoint(&self) -> Url {
        Url::parse(&format!("http://{}", self.address)).unwrap()
    }

    pub fn has_exited(&mut self) -> Option<ExitStatus> {
        self.process.try_wait().expect("Failed to get orchestrator node exit status")
    }

    pub async fn wait_till_started(&mut self) {
        let mut attempts = CONNECTION_ATTEMPTS;
        loop {
            match TcpStream::connect(&self.address).await {
                Ok(_) => return,
                Err(err) => {
                    if let Some(status) = self.has_exited() {
                        panic!("Orchestrator node exited early with {}", status);
                    }
                    if attempts == 0 {
                        panic!("Failed to connect to {}: {}", self.address, err);
                    }
                }
            };

            attempts -= 1;
            tokio::time::sleep(Duration::from_millis(CONNECTION_ATTEMPT_DELAY_MS)).await;
        }
    }
}
