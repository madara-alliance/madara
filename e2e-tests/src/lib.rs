pub mod mongodb;
pub mod node;

use std::net::TcpListener;
use std::path::{Path, PathBuf};

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

fn get_repository_root() -> PathBuf {
    let manifest_path = Path::new(&env!("CARGO_MANIFEST_DIR"));
    let repository_root = manifest_path.parent().expect("Failed to get parent directory of CARGO_MANIFEST_DIR");
    repository_root.to_path_buf()
}
