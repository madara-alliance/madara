use std::{
    io::Read,
    net::SocketAddr,
};

use axum::http::StatusCode;

use hyper::{body::Buf, Body, Request};

use rstest::*;

use orchestrator::{
    queue::init_consumers,
    config::Config,
    routes::app_router,
    utils::env_utils::get_env_var_or_default,
};

use super::common::get_or_init_config;

#[fixture]
pub async fn setup_server(
    #[future]
    #[with( String::from("http://localhost:9944") )]
    get_or_init_config: &'static dyn Config
) -> SocketAddr {
    let _config = get_or_init_config.await;

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
async fn test_health_endpoint(#[future] setup_server: SocketAddr) {
    let addr = setup_server.await;


    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/health", addr)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status().as_str(), StatusCode::OK.as_str());

    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let mut buf = String::new();
    let res = body.reader().read_to_string(&mut buf).unwrap();
    assert_eq!(res, 2);
}

#[rstest]
#[tokio::test]
async fn test_init_consumer() {
    assert!(init_consumers().await.is_ok());
}
