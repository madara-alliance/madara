use clap::ValueEnum;

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

#[derive(Clone, Debug)]
pub enum Cors {
    /// All hosts allowed.
    All,
    /// Only hosts on the list are allowed.
    List(Vec<String>),
}

fn parse_cors(s: &str) -> Result<Cors> {
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

    if is_all { Ok(Cors::All) } else { Ok(Cors::List(origins)) }
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
    /// Same as `--rpc-external`, but enables `--rpc-methods unsafe`
    #[arg(long)]
    pub unsafe_rpc_external: bool,

    /// RPC methods to expose.
    /// - `unsafe`: Exposes every RPC method.
    /// - `safe`: Exposes only a safe subset of RPC methods, denying unsafe RPC methods.
    /// - `auto`: Acts as `safe` if RPC is served externally, e.g. when `--rpc--external` is passed,
    ///   otherwise acts as `unsafe`.
    #[arg(
		long,
		value_name = "METHOD SET",
		value_enum,
		ignore_case = true,
		default_value_t = RpcMethods::Auto,
		verbatim_doc_comment
	)]
    pub rpc_methods: RpcMethods,

    /// Set the the maximum RPC request payload size for both HTTP and WS in megabytes.
    #[arg(long, default_value_t = RPC_DEFAULT_MAX_REQUEST_SIZE_MB)]
    pub rpc_max_request_size: u32,

    /// Set the the maximum RPC response payload size for both HTTP and WS in megabytes.
    #[arg(long, default_value_t = RPC_DEFAULT_MAX_RESPONSE_SIZE_MB)]
    pub rpc_max_response_size: u32,

    /// Set the the maximum concurrent subscriptions per connection.
    #[arg(long, default_value_t = RPC_DEFAULT_MAX_SUBS_PER_CONN)]
    pub rpc_max_subscriptions_per_connection: u32,

    /// Specify JSON-RPC server TCP port.
    #[arg(long, value_name = "PORT")]
    pub rpc_port: Option<u16>,

    /// Maximum number of RPC server connections.
    #[arg(long, value_name = "COUNT", default_value_t = RPC_DEFAULT_MAX_CONNECTIONS)]
    pub rpc_max_connections: u32,

    /// Specify browser Origins allowed to access the HTTP & WS RPC servers.
    /// A comma-separated list of origins (protocol://domain or special `null`
    /// value). Value of `all` will disable origin validation. Default is to
    /// allow localhost.
    #[arg(long, value_name = "ORIGINS", value_parser = parse_cors)]
    pub rpc_cors: Option<Cors>,
}

impl RpcParams {
    fn rpc_interface(
        &self,
    ) -> CliResult<IpAddr> {
        if is_external && is_validator && rpc_methods != RpcMethods::Unsafe {
            return Err(Error::Input(
                "--rpc-external option shouldn't be used if the node is running as \
                 a validator. Use `--unsafe-rpc-external` or `--rpc-methods=unsafe` if you understand \
                 the risks. See the options description for more information."
                    .to_owned(),
            ))
        }
    
        if is_external || is_unsafe_external {
            if rpc_methods == RpcMethods::Unsafe {
                log::warn!(
                    "It isn't safe to expose RPC publicly without a proxy server that filters \
                     available set of RPC methods."
                );
            }
    
            Ok(Ipv4Addr::UNSPECIFIED.into())
        } else {
            Ok(Ipv4Addr::LOCALHOST.into())
        }
    }

    pub fn build(self) -> anyhow::Result<RpcService> {
    }
}
