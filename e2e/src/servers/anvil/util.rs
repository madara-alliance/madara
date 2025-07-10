use crate::servers::server::ServerError;

#[derive(Debug, thiserror::Error)]
pub enum AnvilError {
    #[error("Anvil is not installed on the system")]
    NotInstalled,
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
}

// Configuration specific to Anvil
#[derive(Debug, Clone)]
pub struct AnvilConfig {
    pub port: u16,
    pub host: String,
    pub fork_url: Option<String>,
    pub load_state: Option<String>,
    pub dump_state: Option<String>,
}

impl Default for AnvilConfig {
    fn default() -> Self {
        Self { port: 8545, fork_url: None, load_state: None, dump_state: None, host: "127.0.0.1".to_string() }
    }
}

// Builder for constructing AnvilCMD
pub struct AnvilCMDBuilder {
    port: u16,
    host: String,
    fork_url: Option<String>,
    load_state: Option<String>,
    dump_state: Option<String>,
}

impl AnvilCMDBuilder {
    /// Create a new builder with default values
    pub fn new() -> Self {
        Self { port: 8545, host: "127.0.0.1".to_string(), fork_url: None, load_state: None, dump_state: None }
    }

    /// Set the port (default: 8545)
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Set the host (default: 127.0.0.1)
    pub fn host(mut self, host: String) -> Self {
        self.host = host;
        self
    }

    /// Set the fork URL for forking from an existing network
    pub fn fork_url<S: Into<String>>(mut self, url: S) -> Self {
        self.fork_url = Some(url.into());
        self
    }

    /// Set the database file to load state from
    pub fn load_state<S: Into<String>>(mut self, path: S) -> Self {
        self.load_state = Some(path.into());
        self
    }

    /// Set the database file to dump state to
    pub fn dump_state<S: Into<String>>(mut self, path: S) -> Self {
        self.dump_state = Some(path.into());
        self
    }

    /// Build the final AnvilCMD
    pub fn build(self) -> AnvilConfig {
        AnvilConfig {
            port: self.port,
            host: self.host,
            fork_url: self.fork_url,
            load_state: self.load_state,
            dump_state: self.dump_state,
        }
    }
}

impl Default for AnvilCMDBuilder {
    fn default() -> Self {
        Self::new()
    }
}
