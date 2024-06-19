use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use url::Url;

use crate::get_free_port;

const MONGODB_DEFAULT_PORT: u16 = 27017;
const MONGODB_IMAGE_NAME: &str = "mongo";
const MONGODB_IMAGE_TAG: &str = "8.0-rc";

#[allow(dead_code)]
pub struct MongoDbServer {
    container: ContainerAsync<GenericImage>,
    endpoint: Url,
}

impl MongoDbServer {
    pub async fn run() -> Self {
        let host_port = get_free_port();

        let container = GenericImage::new(MONGODB_IMAGE_NAME, MONGODB_IMAGE_TAG)
            .with_wait_for(WaitFor::message_on_stdout("Waiting for connections"))
            .with_mapped_port(host_port, ContainerPort::Tcp(MONGODB_DEFAULT_PORT))
            .start()
            .await
            .expect("Failed to create docker container");
        Self { container, endpoint: Url::parse(&format!("http://127.0.0.1:{}", host_port)).unwrap() }
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }
}
