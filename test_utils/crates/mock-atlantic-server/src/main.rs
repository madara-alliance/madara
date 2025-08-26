use clap::Parser;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utils_mock_atlantic_server::{MockAtlanticServer, MockServerConfig};

/// Mock Atlantic Server - Simulates the Atlantic prover service for testing
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to run the server on
    #[arg(short, long, default_value_t = 4001, env = "MOCK_ATLANTIC_PORT")]
    port: u16,

    /// Failure rate for simulating failures (0.0 to 1.0)
    #[arg(short = 'f', long, default_value_t = 0.0, value_parser = parse_failure_rate, env = "MOCK_ATLANTIC_FAILURE_RATE")]
    failure_rate: f32,

    /// Processing delay in milliseconds
    #[arg(long, default_value_t = 1000, env = "MOCK_ATLANTIC_PROCESSING_DELAY_MS")]
    processing_delay_ms: u64,

    /// Completion delay in milliseconds
    #[arg(long, default_value_t = 3000, env = "MOCK_ATLANTIC_COMPLETION_DELAY_MS")]
    completion_delay_ms: u64,

    /// Maximum number of jobs to keep in memory
    #[arg(long, default_value_t = 100, env = "MOCK_ATLANTIC_MAX_JOBS")]
    max_jobs_in_memory: usize,

    /// Maximum number of jobs that can run concurrently
    #[arg(long, default_value_t = 10, value_parser = parse_concurrent_jobs, env = "MOCK_ATLANTIC_MAX_CONCURRENT_JOBS")]
    max_concurrent_jobs: usize,

    /// Enable auto-completion of jobs
    #[arg(long, default_value_t = true, env = "MOCK_ATLANTIC_AUTO_COMPLETE")]
    auto_complete_jobs: bool,

    /// Bind address (use 0.0.0.0 to bind to all interfaces)
    #[arg(long, default_value = "127.0.0.1", env = "MOCK_ATLANTIC_BIND_ADDR")]
    bind_addr: String,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "RUST_LOG")]
    log_level: String,
}

fn parse_failure_rate(s: &str) -> Result<f32, String> {
    let rate = s.parse::<f32>().map_err(|e| e.to_string())?;
    if !(0.0..=1.0).contains(&rate) {
        return Err("Failure rate must be between 0.0 and 1.0".to_string());
    }
    Ok(rate)
}

fn parse_concurrent_jobs(s: &str) -> Result<usize, String> {
    let jobs = s.parse::<usize>().map_err(|e| e.to_string())?;
    if jobs == 0 {
        return Err("Max concurrent jobs must be at least 1".to_string());
    }
    if jobs > 1000 {
        return Err("Max concurrent jobs cannot exceed 1000".to_string());
    }
    Ok(jobs)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Initialize tracing with the specified log level
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| args.log_level.clone().into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Validate configuration
    if args.max_concurrent_jobs > args.max_jobs_in_memory {
        return Err(format!(
            "Max concurrent jobs ({}) cannot exceed max jobs in memory ({})",
            args.max_concurrent_jobs, args.max_jobs_in_memory
        ).into());
    }
    
    // Parse bind address
    let bind_addr = args.bind_addr.parse::<std::net::IpAddr>()
        .map_err(|e| format!("Invalid bind address: {}", e))?;
    
    let addr = SocketAddr::from((bind_addr, args.port));
    
    let config = MockServerConfig {
        simulate_failures: args.failure_rate > 0.0,
        processing_delay_ms: args.processing_delay_ms,
        failure_rate: args.failure_rate,
        auto_complete_jobs: args.auto_complete_jobs,
        completion_delay_ms: args.completion_delay_ms,
        max_jobs_in_memory: args.max_jobs_in_memory,
        max_concurrent_jobs: args.max_concurrent_jobs,
    };

    // Display startup information
    println!("╭───────────────────────────────────────╮");
    println!("│     🚀 Mock Atlantic Server           │");
    println!("╰───────────────────────────────────────╯");
    println!();
    println!("Configuration:");
    println!("  📡 Address: {}:{}", bind_addr, args.port);
    println!("  ⏱️  Processing delay: {}ms", args.processing_delay_ms);
    println!("  ⏱️  Completion delay: {}ms", args.completion_delay_ms);
    println!("  📦 Max jobs in memory: {}", args.max_jobs_in_memory);
    println!("  🔄 Max concurrent jobs: {}", args.max_concurrent_jobs);
    println!("  ✅ Auto-complete jobs: {}", args.auto_complete_jobs);
    
    if args.failure_rate > 0.0 {
        println!("  ⚡ Failure simulation: enabled");
        println!("  📊 Failure rate: {:.1}%", args.failure_rate * 100.0);
    } else {
        println!("  ⚡ Failure simulation: disabled");
    }
    
    println!();
    println!("Endpoints:");
    println!("  🔗 Health check: http://{}:{}/is-alive", bind_addr, args.port);
    println!("  📮 Submit job: POST http://{}:{}/atlantic-query", bind_addr, args.port);
    println!("  🔍 Check job: GET http://{}:{}/atlantic-query/{{job_id}}", bind_addr, args.port);
    println!();
    println!("Starting server... Press Ctrl+C to stop");
    println!("────────────────────────────────────────");
    println!();

    let server = MockAtlanticServer::new(addr, config);
    server.run().await?;

    Ok(())
}