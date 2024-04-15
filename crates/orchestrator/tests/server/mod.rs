use std::{
    env, io::Read,
    net::SocketAddr,
};

use axum::http::StatusCode;

use hyper::{body::Buf, Body, Request};

use rstest::*;
use starknet::providers::Provider;

use orchestrator::{
    config::{config, Config},
    routes::app_router,
    utils::env_utils::get_env_var_or_default,
};

#[fixture]
fn rpc_url() -> String {
    String::from("http://localhost:9944")
}

#[fixture]
pub async fn init_valid_config(
    rpc_url: String
) -> &'static Config {
    env::set_var("MADARA_RPC_URL", rpc_url.as_str());
    
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    config().await
}

#[fixture]
pub async fn setup_server(
    #[future] init_valid_config: &Config
) -> SocketAddr {
    let _config = init_valid_config.await;

    let host = get_env_var_or_default("HOST", "127.0.0.1");
    let port = get_env_var_or_default("PORT", "3000").parse::<u16>().expect("PORT must be a u16");
    let address = format!("{}:{}", host, port);

    let listener = tokio::net::TcpListener::bind(address.clone()).await.expect("Failed to get listener");
    let addr = listener.local_addr().unwrap();
    let app = app_router();

    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Failed to start axum server");
    });

    tracing::info!("Listening on http://{}", address);

    addr
}

#[rstest]
#[tokio::test]
async fn test_valid_config(
    #[future] #[from(init_valid_config)] config: &Config
) {
    let config = config.await;

    let result = config.starknet_client().block_number().await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn health(#[future] setup_server: SocketAddr) {
    println!("Setting up the server for health");
    let addr = setup_server.await;
    println!("server for health is set up");


    let client = hyper::Client::new();
    println!("Client created, now making request to health");
    let response = client
        .request(Request::builder().uri(format!("http://{}/health", addr)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status().as_str(), StatusCode::OK.as_str());

    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let mut buf = String::new();
    let res = body.reader().read_to_string(&mut buf).unwrap();
    println!("body health = {}", res);
}