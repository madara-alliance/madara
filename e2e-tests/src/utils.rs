use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use mongodb::bson::doc;
use mongodb::options::{ClientOptions, ServerApi, ServerApiVersion};
use starknet::core::types::StateUpdate;

use crate::MongoDbServer;

pub fn get_repository_root() -> PathBuf {
    let manifest_path = Path::new(&env!("CARGO_MANIFEST_DIR"));
    let repository_root = manifest_path.parent().expect("Failed to get parent directory of CARGO_MANIFEST_DIR");
    repository_root.to_path_buf()
}

pub async fn get_mongo_db_client(mongo_db: &MongoDbServer) -> ::mongodb::Client {
    let mut client_options = ClientOptions::parse(mongo_db.endpoint()).await.expect("Failed to parse MongoDB Url");
    // Set the server_api field of the client_options object to set the version of the Stable API on the
    // client
    let server_api = ServerApi::builder().version(ServerApiVersion::V1).build();
    client_options.server_api = Some(server_api);
    // Get a handle to the cluster
    let client = ::mongodb::Client::with_options(client_options).expect("Failed to create MongoDB client");
    // Ping the server to see if you can connect to the cluster
    client.database("admin").run_command(doc! {"ping": 1}, None).await.expect("Failed to ping MongoDB deployment");

    client
}

pub fn read_state_update_from_file(file_path: &str) -> color_eyre::Result<StateUpdate> {
    // let file_path = format!("state_update_block_no_{}.txt", block_no);
    let mut file = File::open(file_path)?;
    let mut json = String::new();
    file.read_to_string(&mut json)?;
    let state_update: StateUpdate = serde_json::from_str(&json)?;
    Ok(state_update)
}

pub fn vec_u8_to_hex_string(data: &[u8]) -> String {
    let hex_chars: Vec<String> = data.iter().map(|byte| format!("{:02x}", byte)).collect();

    let mut new_hex_chars = hex_chars.join("");
    new_hex_chars = new_hex_chars.trim_start_matches('0').to_string();
    if new_hex_chars.is_empty() {
        "0x0".to_string()
    } else {
        format!("0x{}", new_hex_chars)
    }
}
