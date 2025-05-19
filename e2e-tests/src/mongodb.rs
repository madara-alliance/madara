use orchestrator::types::params::database::DatabaseArgs;
use url::Url;

#[allow(dead_code)]
pub struct MongoDbServer {
    endpoint: Url,
}

impl MongoDbServer {
    pub fn run(mongodb_params: DatabaseArgs) -> Self {
        Self { endpoint: Url::parse(&mongodb_params.connection_uri).expect("Invalid MongoDB connection URI") }
    }

    pub fn endpoint(&self) -> Url {
        self.endpoint.clone()
    }
}
