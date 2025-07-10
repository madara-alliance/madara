use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;
use tokio::net::TcpStream;
use url::Url;
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
}

// Generic server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
    pub connection_attempts: usize,
    pub connection_delay_ms: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { port: 8545, host: "127.0.0.1".to_string(), connection_attempts: 30, connection_delay_ms: 1000 }
    }
}

// Generic server struct that can be used by any service
pub struct Server {
    process: Option<Child>,
    config: ServerConfig,
}

impl Server {
    /// Start a process with the given command and wait for it to be ready
    pub async fn start_process(mut command: Command, config: ServerConfig) -> Result<Self, ServerError> {
        // Set up stdio for the process
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        // Start the process
        let process = command.spawn().map_err(ServerError::StartupFailed)?;

        println!("Starting container with command : {:?}", command);
        let mut server = Self { process: Some(process), config };

        // Wait for the server to be ready
        server.wait_till_started().await?;

        Ok(server)
    }

    /// Get the endpoint URL
    pub fn endpoint(&self) -> Url {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        Url::parse(&format!("http://{}", addr)).unwrap()
    }

    /// Get the port number
    pub fn port(&self) -> u16 {
        self.config.port
    }

    /// Get the host
    pub fn host(&self) -> &str {
        &self.config.host
    }

    /// Get the process ID if the process is still running
    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().map(|p| p.id())
    }

    /// Check if the process has exited
    pub fn has_exited(&mut self) -> Option<ExitStatus> {
        if let Some(ref mut process) = self.process {
            match process.try_wait() {
                Ok(status) => status,
                Err(_) => None,
            }
        } else {
            None
        }
    }

    /// Check if the process is still running
    pub fn is_running(&mut self) -> bool {
        self.process.is_some() && self.has_exited().is_none()
    }

    /// Get a free port
    fn get_free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .unwrap_or(8080) // Fallback port
    }

    /// Wait until the server is ready to accept connections
    async fn wait_till_started(&mut self) -> Result<(), ServerError> {
        println!("Waiting for server to start...");
        let mut attempts = self.config.connection_attempts;
        let addr = format!("{}:{}", self.config.host, self.config.port);

        loop {
            match TcpStream::connect(&addr).await {
                Ok(_) => return Ok(()),
                Err(_) => {
                    // Check if process has exited
                    if let Some(status) = self.has_exited() {
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
    }

    /// Stop the server gracefully
    pub fn stop(&mut self) -> Result<(), ServerError> {
        if let Some(mut process) = self.process.take() {
            // Try to terminate gracefully first
            let pid = process.id();
            let kill_result = Command::new("kill").args(["-s", "TERM", &pid.to_string()]).spawn();

            match kill_result {
                Ok(mut kill_process) => {
                    let _ = kill_process.wait();
                }
                Err(_) => {
                    // If kill command fails, try to kill the process directly
                    let _ = process.kill();
                }
            }

            // Wait for the process to actually exit
            let _ = process.wait();
        }
        Ok(())
    }

    /// Send a signal to the process
    pub fn send_signal(&self, signal: &str) -> Result<(), ServerError> {
        if let Some(ref process) = self.process {
            let pid = process.id();
            Command::new("kill")
                .args(["-s", signal, &pid.to_string()])
                .spawn()
                .map_err(ServerError::Io)?
                .wait()
                .map_err(ServerError::Io)?;
            Ok(())
        } else {
            Err(ServerError::ProcessNotRunning)
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
