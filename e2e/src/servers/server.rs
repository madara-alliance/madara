// use std::process::{ExitStatus, Stdio};
// use std::time::Duration;
// use tokio::net::TcpStream;
// use url::Url;

// use tokio::io::{AsyncBufReadExt, BufReader};
// use tokio::process::{Command, Child};
// use tokio::task::{self, JoinHandle};
// use tokio::sync::oneshot;

// // Custom error type
// #[derive(Debug, thiserror::Error)]
// pub enum ServerError {
//     #[error("Failed to start process: {0}")]
//     StartupFailed(std::io::Error),
//     #[error("Process exited early with status: {0}")]
//     ProcessExited(ExitStatus),
//     #[error("Connection timeout after {0} attempts")]
//     ConnectionTimeout(usize),
//     #[error("IO error: {0}")]
//     Io(#[from] std::io::Error),
//     #[error("Process not running")]
//     ProcessNotRunning,
//     #[error("Log monitoring error: {0}")]
//     LogMonitorError(String),
// }

// // Generic server configuration
// #[derive(Debug, Clone)]
// pub struct ServerConfig {
//     pub port: u16,
//     pub host: String,
//     pub connection_attempts: usize,
//     pub connection_delay_ms: u64,
//     // New configuration options for improved reliability
//     pub per_connection_timeout_ms: u64,
//     pub graceful_shutdown_timeout_ms: u64,
//     pub max_log_line_length: usize,
//     pub enable_process_monitoring: bool,
// }

// impl Default for ServerConfig {
//     fn default() -> Self {
//         Self {
//             port: 8545,
//             host: "127.0.0.1".to_string(),
//             connection_attempts: 30,
//             connection_delay_ms: 1000,
//             per_connection_timeout_ms: 5000,      // 5 seconds per connection attempt
//             graceful_shutdown_timeout_ms: 10000,  // 10 seconds for graceful shutdown
//             max_log_line_length: 4096,            // 4KB max log line length
//             enable_process_monitoring: true,      // Enable background process monitoring
//         }
//     }
// }

// // Generic server struct that can be used by any service
// pub struct Server {
//     process: Option<Child>,
//     config: ServerConfig,
//     // Improved log monitoring with proper lifecycle management
//     stdout_task: Option<JoinHandle<Result<(), std::io::Error>>>,
//     stderr_task: Option<JoinHandle<Result<(), std::io::Error>>>,
//     // Shutdown coordination channels
//     stdout_shutdown_tx: Option<oneshot::Sender<()>>,
//     stderr_shutdown_tx: Option<oneshot::Sender<()>>,
//     // Process health monitoring
//     process_monitor: Option<JoinHandle<()>>,
//     process_monitor_shutdown_tx: Option<oneshot::Sender<()>>,
// }

// impl Server {
//     /// Start a process with the given command and wait for it to be ready
//     pub async fn start_process(mut command: Command, config: ServerConfig) -> Result<Self, ServerError> {
//         println!("🚀 Starting new server process...");

//         // ✅ FIX 1: Set proper process configuration to prevent hanging
//         command
//             .stdout(Stdio::piped())
//             .stderr(Stdio::piped())
//             .stdin(Stdio::null())  // Prevent hanging on stdin reads
//             .kill_on_drop(true);   // Ensure process cleanup when dropped

//         // Start the process
//         let mut process = command.spawn().map_err(ServerError::StartupFailed)?;
//         let process_id = process.id();

//         println!("✅ Process spawned successfully with PID: {:?}", process_id);
//         println!("📋 Command executed: {:?}", command);

//         // Extract stdout and stderr for log monitoring
//         let stdout = process.stdout.take().ok_or(ServerError::StartupFailed(
//             std::io::Error::new(std::io::ErrorKind::Other, "Failed to capture stdout")
//         ))?;
//         let stderr = process.stderr.take().ok_or(ServerError::StartupFailed(
//             std::io::Error::new(std::io::ErrorKind::Other, "Failed to capture stderr")
//         ))?;

//         println!("📡 Stdout and stderr captured for monitoring");

//         // ✅ FIX 2: Create shutdown coordination channels
//         let (stdout_shutdown_tx, stdout_shutdown_rx) = oneshot::channel();
//         let (stderr_shutdown_tx, stderr_shutdown_rx) = oneshot::channel();

//         println!("🔗 Shutdown coordination channels created");

//         // ✅ FIX 3: Spawn improved log monitoring tasks with proper lifecycle
//         let max_line_length = config.max_log_line_length;
//         let stdout_task = Self::spawn_log_monitor(
//             stdout,
//             "STDOUT",
//             stdout_shutdown_rx,
//             max_line_length
//         );

//         let stderr_task = Self::spawn_log_monitor(
//             stderr,
//             "STDERR",
//             stderr_shutdown_rx,
//             max_line_length
//         );

//         println!("📝 Log monitoring tasks spawned");

//         // ✅ FIX 4: Spawn process health monitor if enabled
//         // TODO: move enable_process_monitoring to specific server config
//         let (process_monitor, process_monitor_shutdown_tx) = if config.enable_process_monitoring {
//             let (monitor_shutdown_tx, monitor_shutdown_rx) = oneshot::channel();
//             let monitor = Self::spawn_process_monitor(process_id, monitor_shutdown_rx);
//             println!("❤️  Process health monitor spawned");
//             (Some(monitor), Some(monitor_shutdown_tx))
//         } else {
//             println!("⏭️  Process health monitoring disabled");
//             (None, None)
//         };

//         let mut server = Self {
//             process: Some(process),
//             config,
//             stdout_task: Some(stdout_task),
//             stderr_task: Some(stderr_task),
//             stdout_shutdown_tx: Some(stdout_shutdown_tx),
//             stderr_shutdown_tx: Some(stderr_shutdown_tx),
//             process_monitor,
//             process_monitor_shutdown_tx,
//         };

//         // ✅ FIX 5: Wait for the server to be ready with improved timeout handling
//         println!("⏳ Waiting for server to become ready...");
//         server.wait_till_started().await?;
//         println!("🎉 Server is ready and accepting connections!");

//         Ok(server)
//     }

//     /// ✅ FIX 3: Improved log monitoring with proper error handling and shutdown
//     fn spawn_log_monitor(
//         stdio: impl tokio::io::AsyncRead + Unpin + Send + 'static,
//         prefix: &'static str,
//         mut shutdown_rx: oneshot::Receiver<()>,
//         max_line_length: usize,
//     ) -> JoinHandle<Result<(), std::io::Error>> {
//         task::spawn(async move {
//             println!("📝 Starting {} log monitor", prefix);
//             let reader = BufReader::new(stdio);
//             let mut lines = reader.lines();
//             let mut line_count = 0;

//             loop {
//                 tokio::select! {
//                     // Check for shutdown signal
//                     _ = &mut shutdown_rx => {
//                         println!("🛑 {} log monitor received shutdown signal (processed {} lines)", prefix, line_count);
//                         break Ok(());
//                     }

//                     // Read next line
//                     line_result = lines.next_line() => {
//                         match line_result {
//                             Ok(Some(line)) => {
//                                 line_count += 1;

//                                 // Prevent memory issues with very long lines
//                                 if line.len() > max_line_length {
//                                     println!("[{}:{}] [TRUNCATED] {}...", prefix, line_count, &line[..100]);
//                                 } else {
//                                     println!("[{}:{}] {}", prefix, line_count, line);
//                                 }

//                                 // Periodic logging statistics
//                                 if line_count % 100 == 0 {
//                                     println!("📊 {} monitor: processed {} lines", prefix, line_count);
//                                 }
//                             }
//                             Ok(None) => {
//                                 // EOF reached - process probably died
//                                 println!("📄 {} monitor: EOF reached after {} lines (process may have exited)", prefix, line_count);
//                                 break Ok(());
//                             }
//                             Err(e) => {
//                                 println!("❌ {} monitor: Error reading line after {} lines: {}", prefix, line_count, e);
//                                 break Err(e);
//                             }
//                         }
//                     }
//                 }
//             }
//         })
//     }

//     /// ✅ FIX 4: Process health monitoring
//     fn spawn_process_monitor(
//         process_id: Option<u32>,
//         mut shutdown_rx: oneshot::Receiver<()>,
//     ) -> JoinHandle<()> {
//         task::spawn(async move {
//             println!("❤️  Starting process health monitor for PID: {:?}", process_id);
//             let mut check_count = 0;

//             loop {
//                 tokio::select! {
//                     _ = &mut shutdown_rx => {
//                         println!("🛑 Process monitor received shutdown signal (performed {} checks)", check_count);
//                         break;
//                     }

//                     _ = tokio::time::sleep(Duration::from_secs(10)) => {
//                         check_count += 1;

//                         if let Some(pid) = process_id {
//                             // Platform-specific process health check
//                             #[cfg(unix)]
//                             {
//                                 use std::process::Command;
//                                 let output = Command::new("kill")
//                                     .args(["-0", &pid.to_string()])  // Signal 0 = check if process exists
//                                     .output();

//                                 match output {
//                                     Ok(result) if result.status.success() => {
//                                         if check_count % 6 == 0 {  // Log every minute (6 * 10 seconds)
//                                             println!("💚 Process {} health check #{}: OK", pid, check_count);
//                                         }
//                                     }
//                                     _ => {
//                                         println!("💀 Process {} health check #{}: FAILED - process may be dead", pid, check_count);
//                                         break;
//                                     }
//                                 }
//                             }

//                             #[cfg(not(unix))]
//                             {
//                                 // For non-Unix systems, we can't easily check process health
//                                 if check_count % 6 == 0 {
//                                     println!("❓ Process {} health check #{}: Platform doesn't support health checks", pid, check_count);
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }

//             println!("🏁 Process health monitor completed after {} checks", check_count);
//         })
//     }

//     /// Get the endpoint URL
//     pub fn endpoint(&self) -> Url {
//         let addr = format!("{}:{}", self.config.host, self.config.port);
//         Url::parse(&format!("http://{}", addr)).unwrap()
//     }

//     /// Get the port number
//     pub fn port(&self) -> u16 {
//         self.config.port
//     }

//     /// Get the host
//     pub fn host(&self) -> &str {
//         &self.config.host
//     }

//     /// Get the process ID if the process is still running
//     pub fn pid(&self) -> Option<u32> {
//         self.process.as_ref().and_then(|p| p.id())
//     }

//     /// ✅ FIX 2: Improved process exit detection with error propagation
//     pub fn has_exited(&mut self) -> Result<Option<ExitStatus>, std::io::Error> {
//         if let Some(ref mut process) = self.process {
//             let result = process.try_wait()?;
//             if let Some(status) = result {
//                 println!("📋 Process exit detected with status: {}", status);
//             }
//             Ok(result)
//         } else {
//             println!("⚠️  No process to check (already taken or never started)");
//             Ok(None)
//         }
//     }

//     /// ✅ FIX 2: Improved running check with better error handling
//     pub fn is_running(&mut self) -> bool {
//         match self.has_exited() {
//             Ok(None) => {
//                 let running = self.process.is_some();
//                 if !running {
//                     println!("⚠️  Process check: No process handle available");
//                 }
//                 running
//             }
//             Ok(Some(status)) => {
//                 println!("❌ Process check: Process has exited with status: {}", status);
//                 false
//             }
//             Err(e) => {
//                 println!("❌ Process check: Error checking process status: {}", e);
//                 false
//             }
//         }
//     }

//     /// Get a free port (utility function)
//     fn get_free_port() -> u16 {
//         std::net::TcpListener::bind("127.0.0.1:0")
//             .and_then(|listener| listener.local_addr())
//             .map(|addr| addr.port())
//             .unwrap_or(8080) // Fallback port
//     }

//     /// ✅ FIX 5: Improved startup detection with per-connection timeouts
//     async fn wait_till_started(&mut self) -> Result<(), ServerError> {
//         println!("⏳ Waiting for server to start on {}:{}...", self.config.host, self.config.port);

//         let addr = format!("{}:{}", self.config.host, self.config.port);
//         let mut attempts = self.config.connection_attempts;
//         let start_time = std::time::Instant::now();
//         let per_connection_timeout = Duration::from_millis(self.config.per_connection_timeout_ms);

//         println!("🔧 Connection parameters:");
//         println!("   - Target: {}", addr);
//         println!("   - Max attempts: {}", self.config.connection_attempts);
//         println!("   - Delay between attempts: {}ms", self.config.connection_delay_ms);
//         println!("   - Timeout per attempt: {}ms", self.config.per_connection_timeout_ms);

//         loop {
//             // ✅ Check if process died before trying to connect
//             match self.has_exited() {
//                 Ok(Some(status)) => {
//                     println!("💀 Process exited early with status: {}", status);
//                     return Err(ServerError::ProcessExited(status));
//                 }
//                 Ok(None) => {
//                     // Process still running, continue
//                 }
//                 Err(e) => {
//                     println!("❌ Error checking process status: {}", e);
//                     return Err(ServerError::Io(e));
//                 }
//             }

//             // ✅ Try to connect with timeout per attempt
//             let attempt_num = self.config.connection_attempts - attempts + 1;
//             println!("🔌 Connection attempt #{} to {}...", attempt_num, addr);

//             match tokio::time::timeout(per_connection_timeout, TcpStream::connect(&addr)).await {
//                 Ok(Ok(_stream)) => {
//                     let elapsed = start_time.elapsed();
//                     println!("🎉 Successfully connected to {} after {:?} (attempt #{})", addr, elapsed, attempt_num);
//                     return Ok(());
//                 }
//                 Ok(Err(e)) => {
//                     println!("❌ Connection attempt #{} failed: {}", attempt_num, e);
//                 }
//                 Err(_) => {
//                     println!("⏰ Connection attempt #{} timed out after {}ms", attempt_num, self.config.per_connection_timeout_ms);
//                 }
//             }

//             if attempts == 0 {
//                 let total_elapsed = start_time.elapsed();
//                 println!("💥 All connection attempts exhausted after {:?}", total_elapsed);
//                 return Err(ServerError::ConnectionTimeout(self.config.connection_attempts));
//             }

//             attempts -= 1;
//             println!("⏸️  Waiting {}ms before next attempt... ({} attempts remaining)",
//                     self.config.connection_delay_ms, attempts);

//             tokio::time::sleep(Duration::from_millis(self.config.connection_delay_ms)).await;
//         }
//     }

//     /// ✅ FIX 6: Greatly improved stop method with proper cleanup coordination
//     pub async fn stop(&mut self) -> Result<(), ServerError> {
//         println!("🛑 Server shutdown initiated...");
//         let shutdown_start = std::time::Instant::now();

//         // Step 1: Signal all monitoring tasks to shut down
//         println!("📢 Signaling monitoring tasks to shut down...");

//         if let Some(stdout_shutdown) = self.stdout_shutdown_tx.take() {
//             if stdout_shutdown.send(()).is_err() {
//                 println!("⚠️  STDOUT monitor already shut down");
//             } else {
//                 println!("✅ STDOUT monitor shutdown signal sent");
//             }
//         }

//         if let Some(stderr_shutdown) = self.stderr_shutdown_tx.take() {
//             if stderr_shutdown.send(()).is_err() {
//                 println!("⚠️  STDERR monitor already shut down");
//             } else {
//                 println!("✅ STDERR monitor shutdown signal sent");
//             }
//         }

//         if let Some(monitor_shutdown) = self.process_monitor_shutdown_tx.take() {
//             if monitor_shutdown.send(()).is_err() {
//                 println!("⚠️  Process monitor already shut down");
//             } else {
//                 println!("✅ Process monitor shutdown signal sent");
//             }
//         }

//         // Step 2: Wait for monitoring tasks to complete (with timeout)
//         println!("⏳ Waiting for monitoring tasks to complete...");
//         let task_timeout = Duration::from_secs(5);

//         if let Some(stdout_task) = self.stdout_task.take() {
//             match tokio::time::timeout(task_timeout, stdout_task).await {
//                 Ok(Ok(Ok(()))) => println!("✅ STDOUT monitor completed successfully"),
//                 Ok(Ok(Err(e))) => println!("⚠️  STDOUT monitor completed with error: {}", e),
//                 Ok(Err(e)) => println!("❌ STDOUT monitor task error: {}", e),
//                 Err(_) => println!("⏰ STDOUT monitor shutdown timed out"),
//             }
//         }

//         if let Some(stderr_task) = self.stderr_task.take() {
//             match tokio::time::timeout(task_timeout, stderr_task).await {
//                 Ok(Ok(Ok(()))) => println!("✅ STDERR monitor completed successfully"),
//                 Ok(Ok(Err(e))) => println!("⚠️  STDERR monitor completed with error: {}", e),
//                 Ok(Err(e)) => println!("❌ STDERR monitor task error: {}", e),
//                 Err(_) => println!("⏰ STDERR monitor shutdown timed out"),
//             }
//         }

//         if let Some(process_monitor) = self.process_monitor.take() {
//             match tokio::time::timeout(task_timeout, process_monitor).await {
//                 Ok(Ok(())) => println!("✅ Process monitor completed successfully"),
//                 Ok(Err(e)) => println!("❌ Process monitor task error: {}", e),
//                 Err(_) => println!("⏰ Process monitor shutdown timed out"),
//             }
//         }

//         // Step 3: Terminate the actual process gracefully, then forcefully if needed
//         if let Some(mut process) = self.process.take() {
//             let pid = process.id();
//             println!("🎯 Terminating process PID: {:?}", pid);

//             if let Some(pid) = pid {
//                 // Try graceful termination first (Unix only)
//                 #[cfg(unix)]
//                 {
//                     println!("📤 Sending SIGTERM to process {}", pid);
//                     let kill_result = std::process::Command::new("kill")
//                         .args(["-TERM", &pid.to_string()])
//                         .spawn();

//                     match kill_result {
//                         Ok(mut kill_process) => {
//                             match kill_process.wait() {
//                                 Ok(status) if status.success() => {
//                                     println!("✅ SIGTERM sent successfully");
//                                 }
//                                 Ok(status) => {
//                                     println!("⚠️  SIGTERM command failed with status: {}", status);
//                                 }
//                                 Err(e) => {
//                                     println!("❌ Error waiting for kill command: {}", e);
//                                 }
//                             }
//                         }
//                         Err(e) => {
//                             println!("❌ Failed to spawn kill command: {}", e);
//                         }
//                     }

//                     // Wait for graceful shutdown
//                     let graceful_timeout = Duration::from_millis(self.config.graceful_shutdown_timeout_ms);
//                     println!("⏳ Waiting up to {:?} for graceful shutdown...", graceful_timeout);

//                     let graceful_start = std::time::Instant::now();
//                     let mut graceful_success = false;

//                     while graceful_start.elapsed() < graceful_timeout {
//                         match process.try_wait() {
//                             Ok(Some(status)) => {
//                                 println!("🎉 Process terminated gracefully with status: {} (took {:?})",
//                                         status, graceful_start.elapsed());
//                                 graceful_success = true;
//                                 break;
//                             }
//                             Ok(None) => {
//                                 // Still running, continue waiting
//                                 tokio::time::sleep(Duration::from_millis(100)).await;
//                                 continue;
//                             }
//                             Err(e) => {
//                                 println!("❌ Error checking process exit: {}", e);
//                                 break;
//                             }
//                         }
//                     }

//                     if !graceful_success {
//                         println!("⚠️  Graceful shutdown timed out, forcing termination...");

//                         // Force kill
//                         println!("💥 Sending SIGKILL to process {}", pid);
//                         let force_kill_result = std::process::Command::new("kill")
//                             .args(["-KILL", &pid.to_string()])
//                             .spawn();

//                         match force_kill_result {
//                             Ok(mut kill_process) => {
//                                 match kill_process.wait() {
//                                     Ok(_) => println!("✅ SIGKILL sent"),
//                                     Err(e) => println!("❌ Error with SIGKILL: {}", e),
//                                 }
//                             }
//                             Err(e) => {
//                                 println!("❌ Failed to send SIGKILL: {}", e);
//                             }
//                         }
//                     }
//                 }

//                 // Platform-independent force kill as final fallback
//                 #[cfg(not(unix))]
//                 {
//                     println!("💥 Force killing process (non-Unix platform)");
//                 }

//                 match process.kill().await {
//                     Ok(status) => {
//                         println!("✅ Process force-killed with status:");
//                     }
//                     Err(e) => {
//                         println!("❌ Error force-killing process: {}", e);
//                         // Don't return error here as the process might already be dead
//                     }
//                 }
//             } else {
//                 println!("⚠️  No PID available for process termination");
//             }
//         } else {
//             println!("⚠️  No process to terminate (already taken or never started)");
//         }

//         let total_shutdown_time = shutdown_start.elapsed();
//         println!("🏁 Server shutdown completed in {:?}", total_shutdown_time);
//         Ok(())
//     }
// }

// impl Drop for Server {
//     fn drop(&mut self) {
//         println!("🗑️  Server Drop called - initiating emergency cleanup");

//         // We can't await in Drop, so we use blocking operations
//         if self.process.is_some() ||
//            self.stdout_task.is_some() ||
//            self.stderr_task.is_some() ||
//            self.process_monitor.is_some() {

//             println!("⚠️  Warning: Server dropped without proper shutdown - forcing cleanup");

//             // Try to signal shutdown (non-blocking)
//             if let Some(stdout_shutdown) = self.stdout_shutdown_tx.take() {
//                 let _ = stdout_shutdown.send(());
//             }
//             if let Some(stderr_shutdown) = self.stderr_shutdown_tx.take() {
//                 let _ = stderr_shutdown.send(());
//             }
//             if let Some(monitor_shutdown) = self.process_monitor_shutdown_tx.take() {
//                 let _ = monitor_shutdown.send(());
//             }

//             // Abort tasks if they're still running
//             if let Some(stdout_task) = self.stdout_task.take() {
//                 stdout_task.abort();
//             }
//             if let Some(stderr_task) = self.stderr_task.take() {
//                 stderr_task.abort();
//             }
//             if let Some(process_monitor) = self.process_monitor.take() {
//                 process_monitor.abort();
//             }

//             // Force kill the process if it's still running
//             if let Some(mut process) = self.process.take() {
//                 if let Some(pid) = process.id() {
//                     println!("💀 Emergency: Force killing process {} from Drop", pid);
//                     let _ = process.kill();
//                 }
//             }
//         }

//         println!("🗑️  Server Drop cleanup completed");
//     }
// }




use std::process::{ ExitStatus, Stdio};
use std::time::Duration;
use tokio::net::TcpStream;
use url::Url;

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
