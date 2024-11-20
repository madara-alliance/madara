pub mod anvil;
pub mod mock_server;
pub mod mongodb;
pub mod node;
pub mod sharp;
pub mod starknet_client;
pub mod utils;

use std::net::TcpListener;

pub use mongodb::MongoDbServer;
pub use node::Orchestrator;
pub use orchestrator::database::mongodb::MongoDb as MongoDbClient;

const MIN_PORT: u16 = 49_152;
const MAX_PORT: u16 = 65_535;

fn get_free_port() -> u16 {
    for port in MIN_PORT..=MAX_PORT {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return listener.local_addr().expect("No local addr").port();
        }
        // otherwise port is occupied
    }
    panic!("No free ports available");
}
