use std::{
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    num::NonZeroU32,
    str::FromStr,
};

use clap::ValueEnum;
use ip_network::IpNetwork;
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
    /// Enable the rpc server
    #[arg(long)]
    pub rpc_disabled: bool,

    /// Listen to all RPC interfaces.
    /// Default is local. Note: not all RPC methods are safe to be exposed publicly.
    #[arg(long)]
    pub rpc_external: bool,

    /// RPC methods to expose.
    #[arg(
		long,
		value_name = "METHOD SET",
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
    #[arg(long)]
    pub rpc_rate_limit: Option<NonZeroU32>,

    /// Disable RPC rate limiting for certain ip addresses.
    ///
    /// Each IP address must be in CIDR notation such as `1.2.3.4/24`.
    #[arg(long, num_args = 1..)]
    pub rpc_rate_limit_whitelisted_ips: Vec<IpNetwork>,

    /// Trust proxy headers for disable rate limiting.
    ///
    /// By default the rpc server will not trust headers such `X-Real-IP`, `X-Forwarded-For` and
    /// `Forwarded` and this option will make the rpc server to trust these headers.
    ///
    /// For instance this may be secure if the rpc server is behind a reverse proxy and that the
    /// proxy always sets these headers.
    #[arg(long)]
    pub rpc_rate_limit_trust_proxy_headers: bool,

    /// Set the maximum RPC request payload size for both HTTP and WS in megabytes.
    #[arg(long, default_value_t = RPC_DEFAULT_MAX_REQUEST_SIZE_MB)]
    pub rpc_max_request_size: u32,

    /// Set the maximum RPC response payload size for both HTTP and WS in megabytes.
    #[arg(long, default_value_t = RPC_DEFAULT_MAX_RESPONSE_SIZE_MB)]
    pub rpc_max_response_size: u32,

    /// Set the maximum concurrent subscriptions per connection.
    #[arg(long, default_value_t = RPC_DEFAULT_MAX_SUBS_PER_CONN)]
    pub rpc_max_subscriptions_per_connection: u32,

    /// Specify JSON-RPC server TCP port.
    #[arg(long, value_name = "PORT", default_value_t = RPC_DEFAULT_PORT)]
    pub rpc_port: u16,

    /// Maximum number of RPC server connections.
    #[arg(long, value_name = "COUNT", default_value_t = RPC_DEFAULT_MAX_CONNECTIONS)]
    pub rpc_max_connections: u32,

    /// The number of messages the RPC server is allowed to keep in memory.
    ///
    /// If the buffer becomes full then the server will not process
    /// new messages until the connected client start reading the
    /// underlying messages.
    ///
    /// This applies per connection which includes both
    /// JSON-RPC methods calls and subscriptions.
    #[arg(long, default_value_t = RPC_DEFAULT_MESSAGE_CAPACITY_PER_CONN)]
    pub rpc_message_buffer_capacity_per_connection: u32,

    /// Disable RPC batch requests
    #[arg(long, alias = "rpc_no_batch_requests", conflicts_with_all = &["rpc_max_batch_request_len"])]
    pub rpc_disable_batch_requests: bool,

    /// Limit the max length per RPC batch request
    #[arg(long, conflicts_with_all = &["rpc_disable_batch_requests"], value_name = "LEN")]
    pub rpc_max_batch_request_len: Option<u32>,

    /// Specify browser *origins* allowed to access the HTTP & WS RPC servers.
    ///
    /// A comma-separated list of origins (protocol://domain or special `null`
    /// value). Value of `all` will disable origin validation. Default is to
    /// allow localhost and <https://polkadot.js.org> origins. When running in
    /// `--dev` mode the default is to allow all origins.
    #[arg(long, value_name = "ORIGINS")]
    pub rpc_cors: Option<Cors>,
}

impl RpcParams {
    pub fn cors(&self) -> Option<Vec<String>> {
        let cors = self.rpc_cors.clone().map(|c| c).unwrap_or_else(|| {
            Cors::List(vec![
                "http://localhost:*".into(),
                "http://127.0.0.1:*".into(),
                "https://localhost:*".into(),
                "https://127.0.0.1:*".into(),
            ])
        });

        match cors {
            Cors::All => None,
            Cors::List(ls) => Some(ls),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        let interface = if self.rpc_external {
            Ipv4Addr::UNSPECIFIED // listen on 0.0.0.0
        } else {
            Ipv4Addr::LOCALHOST
        };
        let rpc_addr = SocketAddr::new(interface.into(), self.rpc_port);
        rpc_addr
    }

    pub fn batch_config(&self) -> BatchRequestConfig {
        let cfg = if self.rpc_disable_batch_requests {
            BatchRequestConfig::Disabled
        } else if let Some(l) = self.rpc_max_batch_request_len {
            BatchRequestConfig::Limit(l)
        } else {
            BatchRequestConfig::Unlimited
        };

        cfg
    }
}
