use std::collections::HashMap;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::net::TcpStream;
use url::Url;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task;

use super::constants::*;



// Custom error type
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("Failed to start process: {0}")]
    StartupFailed(std::io::Error),
    #[error("Process exited early with status: {0}")]
    ProcessExited(ExitStatus),
    #[error("Connection timeout after {0} attempts")]
    ConnectionTimeout(usize),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Process not running")]
    ProcessNotRunning,
    #[error("Endpoint not available")]
    EndpointNotAvailable,
    #[error("Failed to bind to port")]
    PortBindFailed(std::io::Error),
}

// Generic server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub rpc_port: Option<u16>,
    pub connection_attempts: usize,
    pub connection_delay_ms: u64,
    pub buffer_capacity: usize,
    pub service_name: String,
    // stdout, stderr
    pub logs: (bool, bool),
    // Environment variables to pass to the process
    pub env_vars: Option<HashMap<String, String>>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            service_name: String::from(""),
            rpc_port: None,
            connection_attempts: CONNECTION_ATTEMPTS,
            connection_delay_ms: CONNECTION_DELAY_MS,
            buffer_capacity: BUFFER_CAPACITY,
            logs: (true, true),
            env_vars: None,
        }
    }
}

impl ServerConfig {
    /// Helper method to inherit current process environment variables
    pub fn with_current_env(mut self) -> Self {
        let current_env: HashMap<String, String> = std::env::vars().collect();
        self.env_vars = Some(current_env);
        self
    }
}

// Generic server struct that can be used by any service
pub struct Server {
    config: ServerConfig,
    process: Child,
    stdout_task: Option<task::JoinHandle<()>>,
    stderr_task: Option<task::JoinHandle<()>>,
}

impl Server {
    /// Start a process with the given command and wait for it to be ready
    pub async fn start_process(mut command: Command, config: ServerConfig) -> Result<Self, ServerError> {
        println!("🔔 Starting {} service", config.service_name);

        let config = config.with_current_env();

        if config.logs.0 {
            command.stdout(Stdio::piped());
        } else {
            // Suppress stdout if disabled
            command.stdout(Stdio::null());
        }

        if config.logs.1 {
            command.stderr(Stdio::piped());
        } else {
            // Suppress stderr if disabled
            command.stderr(Stdio::null());
        }

        // Start the process
        let mut process = command.spawn().map_err(ServerError::StartupFailed)?;

        let mut stdout_task = None;
        let mut stderr_task = None;

        // Extract stdout and stderr for log monitoring
        if config.logs.0 {
            let stdout = process.stdout.take().ok_or(ServerError::StartupFailed(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to capture stdout",
            )))?;

            let service_name = config.service_name.clone();
            let stdout_task_inner = task::spawn(async move {
                // Keeping a large buffer capacity for stdout, to not have buffer overflow
                let reader = BufReader::with_capacity(config.buffer_capacity, stdout);
                let mut lines = reader.lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    println!("[STDOUT] [{}] {}", service_name, line);

                    // Flush immediately to prevent backing up
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
            });
            stdout_task = Some(stdout_task_inner);
        }

        if config.logs.1 {
            let stderr = process.stderr.take().ok_or(ServerError::StartupFailed(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to capture stderr",
            )))?;

            let service_name = config.service_name.clone();
            let stderr_task_inner = task::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    println!("[STDERR] [{}] {}", service_name, line);
                    // No need for flush, since we stop the service on errors
                }
            });

            stderr_task = Some(stderr_task_inner)
        }

        let service_name = config.service_name.clone();
        let has_rpc_endpoint = config.rpc_port.is_some();

        let mut server = Self { process, config, stdout_task, stderr_task };

        // We wait & validate only if the service has an API endpoint
        // e.g : Skips for Bootstrapper and Orchestrator setup
        if has_rpc_endpoint {
            println!("🔔 Waiting for {} server to be ready", service_name);
            server.wait_till_started().await?;
        }

        println!("😁 {} Server is ready", service_name);

        Ok(server)
    }

    /// Get the endpoint URL
    pub fn endpoint(&self) -> Option<Url> {
        if let Some(rpc_port) = &self.config.rpc_port {
            Url::parse(&format!("http://{}:{}", DEFAULT_SERVICE_HOST, rpc_port)).ok()
        } else {
            None
        }
    }

    /// Get the process ID if the process is still running
    pub fn pid(&self) -> Option<u32> {
        self.process.id()
    }

    /// Check if the process has exited
    pub fn has_exited(&mut self) -> Result<Option<ExitStatus>, ServerError> {
        match self.process.try_wait() {
            Ok(status) => Ok(status),
            Err(_) => Err(ServerError::ProcessNotRunning),
        }
    }

    /// Wait until the server is ready to accept connections
    async fn wait_till_started(&mut self) -> Result<(), ServerError> {
        let mut attempts = self.config.connection_attempts;
        let addr = self.endpoint();

        if let Some(addr) = addr {
            // Extract just the socket address from the URL
            let socket_addr = if let Some(host) = addr.host_str() {
                let port = addr.port().expect("Port cannot be None!");
                format!("{}:{}", host, port)
            } else {
                return Err(ServerError::EndpointNotAvailable);
            };

            loop {
                match TcpStream::connect(&socket_addr).await {
                    Ok(_) => return Ok(()),
                    Err(_) => {
                        // Check if process has exited
                        if let Some(status) = self.has_exited()? {
                            return Err(ServerError::ProcessExited(status));
                        }

                        if attempts == 0 {
                            return Err(ServerError::ConnectionTimeout(self.config.connection_attempts));
                        }
                    }
                }

                attempts -= 1;
                tokio::time::sleep(Duration::from_millis(self.config.connection_delay_ms)).await;
            }
        } else {
            Err(ServerError::EndpointNotAvailable)
        }
    }

    /// Stop the server gracefully
    /// This function should not be turned async since it's used within Drop.
    /// And drop impl doesn't handle async functions.
    pub fn stop(&mut self) -> Result<(), ServerError> {
        if !self.config.rpc_port.is_some() {
            return Ok(());
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        // Check if already exited
        if self.has_exited().is_some() {
            return Ok(());
        }

        // Try to terminate gracefully first
        let pid = self.process.id();
        if let Some(pid) = pid {
            let kill_result = Command::new("kill").args(["-s", "TERM", &pid.to_string()]).spawn();

            match kill_result {
                Ok(mut kill_process) => {
                    let _ = kill_process.wait();
                }
                Err(_) => {
                    // If kill command fails, try to kill the process directly
                    let _ = self.process.kill();
                }
            }

            // Wait for the process to actually exit
            let _ = self.process.wait();
        }

        if let Some(task) = self.stdout_task.take() {
            task.abort();
        }
        if let Some(task) = self.stderr_task.take() {
            task.abort();
        }

        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
