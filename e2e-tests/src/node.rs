use std::io::{BufRead, BufReader};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Duration;

use tokio::net::TcpStream;
use url::Url;

use crate::get_free_port;
use crate::utils::get_repository_root;

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
    pub fn run(envs: Vec<(String, String)>) -> Self {
        let port = get_free_port();
        let address = format!("127.0.0.1:{}", port);
        let repository_root = &get_repository_root();

        std::env::set_current_dir(repository_root).expect("Failed to change working directory");

        let port_str = format!("{}", port);
        let envs = [envs, vec![("PORT".to_string(), port_str)]].concat();

        let mut command = Command::new("cargo");
        command
            .arg("run")
            .arg("--bin")
            .arg("orchestrator")
            .arg("--features")
            .arg("testing")
            .current_dir(repository_root)
            .envs(envs)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut process = command.spawn().expect("Failed to start process");

        // Capture and print stdout
        let stdout = process.stdout.take().expect("Failed to capture stdout");
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            reader.lines().for_each(|line| {
                if let Ok(line) = line {
                    println!("STDOUT: {}", line);
                }
            });
        });

        // Capture and print stderr
        let stderr = process.stderr.take().expect("Failed to capture stderr");
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            reader.lines().for_each(|line| {
                if let Ok(line) = line {
                    eprintln!("STDERR: {}", line);
                }
            });
        });

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
