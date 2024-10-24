use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::str::FromStr;

use clap::ValueEnum;
use jsonrpsee::server::BatchRequestConfig;

/// Available RPC methods.
#[derive(Debug, Copy, Clone, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum RpcMethods {
    /// Expose every RPC method only when RPC is listening on `localhost`,
    /// otherwise serve only safe RPC methods.
    Auto,
    /// Allow only a safe subset of RPC methods.
    Safe,
    /// Expose every RPC method (even potentially unsafe ones).
    Unsafe,
}

/// The default port.
pub const RPC_DEFAULT_PORT: u16 = 9944;
/// The default max number of subscriptions per connection.
pub const RPC_DEFAULT_MAX_SUBS_PER_CONN: u32 = 1024;
/// The default max request size in MB.
pub const RPC_DEFAULT_MAX_REQUEST_SIZE_MB: u32 = 15;
/// The default max response size in MB.
pub const RPC_DEFAULT_MAX_RESPONSE_SIZE_MB: u32 = 15;
/// The default number of connection..
pub const RPC_DEFAULT_MAX_CONNECTIONS: u32 = 100;
/// The default number of messages the RPC server
/// is allowed to keep in memory per connection.
pub const RPC_DEFAULT_MESSAGE_CAPACITY_PER_CONN: u32 = 64;

#[derive(Clone, Debug)]
pub enum Cors {
    /// All hosts allowed.
    All,
    /// Only hosts on the list are allowed.
    List(Vec<String>),
}

impl FromStr for Cors {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut is_all = false;
        let mut origins = Vec::new();
        for part in s.split(',') {
            match part {
                "all" | "*" => {
                    is_all = true;
                    break;
                }
                other => origins.push(other.to_owned()),
            }
        }

        if is_all {
            Ok(Cors::All)
        } else {
            Ok(Cors::List(origins))
        }
    }
}

#[derive(Clone, Debug, clap::Args)]
pub struct RpcParams {
    /// Disable the RPC server.
    #[arg(env = "MADARA_RPC_DISABLED", long, alias = "no-rpc")]
    pub rpc_disabled: bool,

    /// Listen to all network interfaces. This usually means that the RPC server will be accessible externally.
    /// Please note that some endpoints should not be exposed to the outside world - by default, enabling remote access
    /// will disable these endpoints. To re-enable them, use `--rpc-methods unsafe`
    #[arg(env = "MADARA_RPC_EXTERNAL", long)]
    pub rpc_external: bool,

    /// RPC methods to expose.
    #[arg(
		env = "MADARA_RPC_METHODS",
		long,
		value_name = "METHOD",
		value_enum,
		ignore_case = true,
		default_value_t = RpcMethods::Auto,
		verbatim_doc_comment
	)]
    pub rpc_methods: RpcMethods,

    /// RPC rate limiting (calls/minute) for each connection.
    ///
    /// This is disabled by default.
    ///
    /// For example `--rpc-rate-limit 10` will maximum allow
    /// 10 calls per minute per connection.
    #[arg(env = "MADARA_RPC_RATE_LIMIT", long)]
    pub rpc_rate_limit: Option<NonZeroU32>,

    /// Set the maximum RPC request payload size for both HTTP and WebSockets in megabytes.
    #[arg(env = "MADARA_RPC_MAX_REQUEST_SIZE", long, default_value_t = RPC_DEFAULT_MAX_REQUEST_SIZE_MB)]
    pub rpc_max_request_size: u32,

    /// Set the maximum RPC response payload size for both HTTP and WebSockets in megabytes.
    #[arg(env = "MADARA_RPC_MAX_RESPONSE_SIZE", long, default_value_t = RPC_DEFAULT_MAX_RESPONSE_SIZE_MB)]
    pub rpc_max_response_size: u32,

    /// Set the maximum concurrent subscriptions per connection.
    #[arg(env = "MADARA_RPC_MAX_SUBSCRIPTIONS_PER_CONNECTION", long, default_value_t = RPC_DEFAULT_MAX_SUBS_PER_CONN)]
    pub rpc_max_subscriptions_per_connection: u32,

    /// The RPC port to listen at.
    #[arg(env = "MADARA_RPC_PORT", long, value_name = "PORT", default_value_t = RPC_DEFAULT_PORT)]
    pub rpc_port: u16,

    /// Maximum number of RPC server connections at a given time.
    #[arg(env = "MADARA_RPC_MAX_CONNECTIONS", long, value_name = "COUNT", default_value_t = RPC_DEFAULT_MAX_CONNECTIONS)]
    pub rpc_max_connections: u32,

    /// The maximum number of messages that can be kept in memory at a given time, per connection.
    /// The server enforces backpressure, and this buffering is useful when the client cannot keep up with our server.
    #[arg(env = "MADARA_RPC_MESSAGE_BUFFER_CAPACITY_PER_CONNECTION", long, default_value_t = RPC_DEFAULT_MESSAGE_CAPACITY_PER_CONN)]
    pub rpc_message_buffer_capacity_per_connection: u32,

    /// Disable RPC batch requests.
    #[arg(env = "MADARA_RPC_DISABLE_BATCH_REQUESTS", long, alias = "rpc_no_batch_requests", conflicts_with_all = &["rpc_max_batch_request_len"])]
    pub rpc_disable_batch_requests: bool,

    /// Limit the max length for an RPC batch request.
    #[arg(env = "MADARA_RPC_MAX_BATCH_REQUEST_LEN", long, conflicts_with_all = &["rpc_disable_batch_requests"], value_name = "LEN")]
    pub rpc_max_batch_request_len: Option<u32>,

    /// Specify browser *origins* allowed to access the HTTP & WebSocket RPC
    /// servers.
    ///
    /// For most purposes, an origin can be thought of as just `protocol://domain`.
    /// Default behavior depends on `rpc_external`:
    ///
    ///     - If rpc_external is set, CORS will default to allow all incoming
    ///     addresses.
    ///     - If rpc_external is not set, CORS will default to allow only
    ///     connections from `localhost`.
    ///
    /// > If the rpcs are permissive, the same will be true for core, and
    /// > vise-versa.
    ///
    /// This argument is a comma separated list of origins, or the special `all`
    /// value.
    ///
    /// Learn more about CORS and web security at
    /// <https://developer.mozilla.org/en-US/docs/Web/HTTP/CORS>.
    #[arg(env = "MADARA_RPC_CORS", long, value_name = "ORIGINS")]
    pub rpc_cors: Option<Cors>,
}

impl RpcParams {
    pub fn cors(&self) -> Option<Vec<String>> {
        let cors = self.rpc_cors.clone().unwrap_or_else(|| {
            if self.rpc_external {
                Cors::All
            } else {
                Cors::List(vec![
                    "http://localhost:*".into(),
                    "http://127.0.0.1:*".into(),
                    "https://localhost:*".into(),
                    "https://127.0.0.1:*".into(),
                ])
            }
        });

        match cors {
            Cors::All => None,
            Cors::List(ls) => Some(ls),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        let listen_addr = if self.rpc_external {
            Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
        } else {
            Ipv4Addr::LOCALHOST
        };

        SocketAddr::new(listen_addr.into(), self.rpc_port)
    }

    pub fn batch_config(&self) -> BatchRequestConfig {
        if self.rpc_disable_batch_requests {
            BatchRequestConfig::Disabled
        } else if let Some(l) = self.rpc_max_batch_request_len {
            BatchRequestConfig::Limit(l)
        } else {
            BatchRequestConfig::Unlimited
        }
    }
}
