use std::env;
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use utils_mock_atlantic_server::{MockAtlanticServer, MockServerConfig};

const DEFAULT_SERVER_PORT: u16 = 4001;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args: Vec<String> = env::args().collect();

    // Parse command line arguments
    let port = if args.len() > 1 { args[1].parse::<u16>().unwrap_or(DEFAULT_SERVER_PORT) } else { DEFAULT_SERVER_PORT };

    let failure_rate = if args.len() > 2 { args[2].parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0) } else { 0.0 };

    let simulate_failures = failure_rate > 0.0;

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let config = MockServerConfig {
        simulate_failures,
        processing_delay_ms: 1000,
        failure_rate,
        auto_complete_jobs: true,
        completion_delay_ms: 3000,
    };

    println!("🚀 Mock Atlantic Server");
    println!("📡 Port: {}", port);
    println!("⚡ Failure simulation: {}", if simulate_failures { "enabled" } else { "disabled" });
    if simulate_failures {
        println!("📊 Failure rate: {:.1}%", failure_rate * 100.0);
    }
    println!("🔗 Health check: http://127.0.0.1:{}/is-alive", port);
    println!();

    let server = MockAtlanticServer::new(addr, config);
    server.run().await?;

    Ok(())
}
