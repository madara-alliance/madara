

macro_rules! spawn_consumer {
    ($queue_type:expr, $handler:expr, $consume_function:expr, $config:expr) => {
        let config_clone = $config.clone();
        tokio::spawn(async move {
            loop {
                match $consume_function($queue_type, $handler, config_clone.clone()).await {
                    Ok(_) => {}
                    Err(e) => tracing::error!("Failed to consume from queue {:?}. Error: {:?}", $queue_type, e),
                }
                sleep(Duration::from_millis(500)).await;
            }
        });
    };
}