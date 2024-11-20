use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use strum_macros::Display;
use tokio::net::TcpStream;
use url::Url;

use crate::get_free_port;
use crate::utils::get_repository_root;

const CONNECTION_ATTEMPTS: usize = 720;
const CONNECTION_ATTEMPT_DELAY_MS: u64 = 1000;

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

#[derive(Display, Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorMode {
    #[strum(serialize = "run")]
    Run,
    #[strum(serialize = "setup")]
    Setup,
}
impl Orchestrator {
    pub fn new(mode: OrchestratorMode, mut envs: Vec<(String, String)>) -> Option<Self> {
        let repository_root = &get_repository_root();
        let mut address = String::new();
        std::env::set_current_dir(repository_root).expect("Failed to change working directory");

        let is_run_mode = mode == OrchestratorMode::Run;
        let mode_str = mode.to_string();

        println!("Running orchestrator in {} mode", mode_str);

        // Configure common command arguments
        let mut command = Command::new("cargo");
        command
            .arg("run")
            .arg("--release")
            .arg("--bin")
            .arg("orchestrator")
            .arg("--features")
            .arg("testing")
            .arg(mode_str)
            .arg("--aws")
            .arg("--aws-s3")
            .arg("--aws-sqs")
            .arg("--aws-sns");

        // Add event bridge arg only for setup mode
        if is_run_mode {
            command.arg("--settle-on-ethereum");
            command.arg("--da-on-ethereum");
            command.arg("--sharp");
            command.arg("--mongodb");

            let port = get_free_port();
            let addr = format!("127.0.0.1:{}", port);
            envs.push(("MADARA_ORCHESTRATOR_PORT".to_string(), port.to_string()));
            address = addr;

            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            command.arg("--aws-event-bridge");

            // For setup mode, inherit the stdio to show output directly
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }

        command.current_dir(repository_root).envs(envs);

        let mut process = command.spawn().expect("Failed to start process");

        if is_run_mode {
            let stdout = process.stdout.take().expect("Failed to capture stdout");
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                reader.lines().for_each(|line| {
                    if let Ok(line) = line {
                        println!("STDOUT: {}", line);
                    }
                });
            });

            let stderr = process.stderr.take().expect("Failed to capture stderr");
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                reader.lines().for_each(|line| {
                    if let Ok(line) = line {
                        eprintln!("STDERR: {}", line);
                    }
                });
            });
            Some(Self { process, address })
        } else {
            // Wait for the process to complete and get its exit status
            let status = process.wait().expect("Failed to wait for process");
            if status.success() {
                println!("Orchestrator cloud setup completed ✅");
            } else {
                // Get the exit code if available
                if let Some(code) = status.code() {
                    println!("Orchestrator cloud setup failed with exit code: {}", code);
                } else {
                    println!("Orchestrator cloud setup terminated by signal");
                }
            }
            None
        }
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
