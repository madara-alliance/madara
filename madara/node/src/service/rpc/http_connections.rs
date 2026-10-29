//! Close HTTP transport tasks on stop, independently of the admitted RPC method owners.
use std::future::Future;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Default)]
pub(super) struct HttpConnections {
    stop: CancellationToken,
    tasks: TaskTracker,
}

impl HttpConnections {
    /// Abort partial uploads, idle connections and responses whose clients stopped reading.
    /// Admitted database calls belong to the separate RPC tracker and continue to completion.
    pub(super) fn close(&self) {
        self.stop.cancel();
        self.tasks.close();
    }

    pub(super) async fn wait(&self) {
        self.tasks.wait().await;
    }
}

impl<F> hyper::rt::Executor<F> for HttpConnections
where
    F: Future + Send + 'static,
    F::Output: Send,
{
    fn execute(&self, connection: F) {
        let stop = self.stop.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                biased;
                _ = stop.cancelled() => {},
                _ = connection => {},
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{convert::Infallible, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[rstest::rstest]
    #[case::unfinished_upload(true)]
    #[case::unread_response(false)]
    #[tokio::test]
    async fn rpc_shutdown_closes_stalled_http_connections(#[case] unfinished_upload: bool) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let connections = HttpConnections::default();
        let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
        let make_service = hyper::service::make_service_fn(move |_| {
            let started = started.clone();
            async move {
                Ok::<_, Infallible>(hyper::service::service_fn(move |request: hyper::Request<hyper::Body>| {
                    let started = started.clone();
                    async move {
                        started.send(()).unwrap();
                        hyper::body::to_bytes(request.into_body()).await?;
                        // Exceed the socket send buffer so the unread response cannot finish.
                        Ok::<_, hyper::Error>(hyper::Response::new(hyper::Body::from(vec![0; 16 * 1024 * 1024])))
                    }
                }))
            }
        });
        let stop = connections.stop.clone();
        let server = hyper::Server::from_tcp(listener)
            .unwrap()
            .executor(connections.clone())
            .serve(make_service)
            .with_graceful_shutdown(async move { stop.cancelled().await });
        let server = tokio::spawn(server);
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        if unfinished_upload {
            // Keep the socket open after sending only one of the declared 10,000 bytes.
            client.write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 10000\r\n\r\n{").await.unwrap();
        } else {
            client.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
            client.read_exact(&mut [0; 1]).await.unwrap();
        }
        ready.recv().await.unwrap();
        assert!(!server.is_finished());
        connections.close();
        tokio::time::timeout(Duration::from_secs(1), async {
            server.await.unwrap().unwrap();
            connections.wait().await;
        })
        .await
        .unwrap();
        drop(client);
    }
}
