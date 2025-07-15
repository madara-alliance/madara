use std::process::{ ExitStatus, Stdio};
use std::time::Duration;
use tokio::net::TcpStream;
use url::Url;
use serde_json::json;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Command, Child};
use tokio::task;

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
    pub skip_wait_for_ready: bool,
    pub connection_attempts: usize,
    pub connection_delay_ms: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { port: 0, host: "127.0.0.1".to_string(), skip_wait_for_ready: false, connection_attempts: 30, connection_delay_ms: 1000 }
    }
}

// Generic server struct that can be used by any service
pub struct Server {
    process: Option<Child>,
    config: ServerConfig,
    stdout_task: Option<task::JoinHandle<()>>,
    stderr_task: Option<task::JoinHandle<()>>,
}

impl Server {
    /// Start a process with the given command and wait for it to be ready
    pub async fn start_process(mut command: Command, config: ServerConfig) -> Result<Self, ServerError> {
        // Set up stdio for the process
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        // Start the process
        let mut process = command.spawn().map_err(ServerError::StartupFailed)?;


        // Extract stdout and stderr for log monitoring
        let stdout = process.stdout.take().ok_or(ServerError::StartupFailed(
            std::io::Error::new(std::io::ErrorKind::Other, "Failed to capture stdout")
        ))?;
        let stderr = process.stderr.take().ok_or(ServerError::StartupFailed(
            std::io::Error::new(std::io::ErrorKind::Other, "Failed to capture stderr")
        ))?;

        let stdout_task = task::spawn(async move {
            let reader = BufReader::with_capacity(65536, stdout);  // ✅ Large buffer
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                println!("[STDOUT] {}", line);

                // ✅ Critical: Flush immediately to prevent backing up
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        });

        let stderr_task = task::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                println!("[STDERR] {}", line);
            }
        });

        let flag = config.skip_wait_for_ready;
        let mut server = Self {
            process: Some(process),
            config,
            stdout_task: Some(stdout_task),
            stderr_task: Some(stderr_task),
        };

        // Wait for the server to be ready
        println!("🔔 Waiting for server to be ready");
        if !flag {
            server.wait_till_started().await?;
        }
        println!("😁 Server is ready");


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
        self.process.as_ref().and_then(|p| p.id())
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
        if self.config.skip_wait_for_ready {
            return Ok(());
        }
        println!("‼️ Server was asked to stop !");

        if let Some(mut process) = self.process.take() {
            // Try to terminate gracefully first
            let pid = process.id();
            let kill_result = Command::new("kill").args(["-s", "TERM", &pid.unwrap().to_string()]).spawn();

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

    // /// Send a signal to the process
    // pub fn send_signal(&self, signal: &str) -> Result<(), ServerError> {
    //     if let Some(ref process) = self.process {
    //         let pid = process.id();
    //         Command::new("kill")
    //             .args(["-s", signal, &pid.to_string()])
    //             .spawn()
    //             .map_err(ServerError::Io)?
    //             .wait()
    //             .map_err(ServerError::Io)?;
    //         Ok(())
    //     } else {
    //         Err(ServerError::ProcessNotRunning)
    //     }
    // }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}


#[derive(Debug, thiserror::Error)]
pub enum NodeRpcError{
    #[error("Invalid response")]
    InvalidResponse,


}

pub trait NodeRpcMethods {
    fn get_endpoint(&self) -> Url;

    /// Fetches the latest block number from the Starknet RPC endpoint.
    ///
    /// Returns:
    /// - `Ok(-1)` when no blocks have been mined yet (RPC returns "Block not found" error with code 24)
    /// - `Ok(block_number)` when blocks exist
    /// - `Err(NodeRpcError::InvalidResponse)` for other RPC errors or parsing failures
    ///
    /// Note: The -1 return value indicates that the blockchain is in an initial state
    /// with no blocks mined, which is common when first starting a node or testnet.
    async fn get_latest_block_number(&self) -> Result<i64, NodeRpcError> {
        let url = self.get_endpoint();
        let client = reqwest::Client::new();
        let response = client.post(url)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .json(&json!({
                "id": 1,
                "jsonrpc": "2.0",
                "method": "starknet_blockNumber",
                "params": []
            }))
            .send()
            .await
            .map_err(|_| NodeRpcError::InvalidResponse)?;

        let json = response.json::<serde_json::Value>().await
            .map_err(|_| NodeRpcError::InvalidResponse)?;

        // Check if there's an error in the JSON-RPC response
        if let Some(error) = json.get("error") {
            // Check for specific "Block not found" error (code 24)
            if let (Some(code), Some(message)) = (error.get("code"), error.get("message")) {
                if code.as_u64() == Some(24) &&
                   message.as_str().map(|s| s.contains("Block not found")).unwrap_or(false) {
                    println!("No blocks mined yet, returning -1");
                    return Ok(-1);
                }
            }

            println!("RPC Error: {:?}", error);
            return Err(NodeRpcError::InvalidResponse);
        }

        // Extract block number directly from result (it's just an integer now)
        let block_number = json.get("result")
            .and_then(|v| v.as_u64())
            .ok_or(NodeRpcError::InvalidResponse)?;

        let block_num_i64 = block_number as i64;

        println!("Madara Block Number: {}", block_num_i64);

        Ok(block_num_i64)
    }

}
