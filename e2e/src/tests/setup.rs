use rstest::*;
use crate::setup::ChainSetup;
use crate::setup::SetupConfigBuilder;


// Async fixture that takes arguments from the test
#[fixture]
pub async fn setup_chain(#[default("")] test_name: &str) -> ChainSetup {
    // Load environment variables from .env.e2e file
    // This loads .env.e2e from the current directory
    dotenvy::from_filename_override(".env.e2e").expect("Failed to load the .env file");

    // Setting Config!
    println!("Running {}", test_name);
    let setup_config = SetupConfigBuilder::new(None).test_config_l2(test_name).unwrap();
    println!("Running setup");

    // Running Chain
    let mut setup_struct = ChainSetup::new(setup_config).unwrap();
    match setup_struct.setup(test_name).await {
        Ok(()) => println!("✅ Setup completed successfully"),
        Err(e) => {
            println!("❌ Setup failed: {}", e);
            panic!("Setup failed: {}", e);
        }
    }

    setup_struct
}


use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct JsonReader {
    data: Map<String, Value>,
}

impl JsonReader {
    /// Create a new JsonReader from a JSON file path
    pub fn new<P: AsRef<Path>>(file_path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(file_path)?;
        let json_value: Value = serde_json::from_str(&content)?;

        match json_value {
            Value::Object(map) => Ok(JsonReader { data: map }),
            _ => Err("JSON file must contain an object at root level".into()),
        }
    }

    /// Get a value by key, returns None if key doesn't exist
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    /// Get a value by key and attempt to convert to string
    pub fn get_string(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    /// Get a value by key and attempt to convert to i64
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key)?.as_i64()
    }

    /// Get a value by key and attempt to convert to f64
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }

    /// Get a value by key and attempt to convert to bool
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    /// Check if a key exists
    pub fn contains_key(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    /// Get all keys
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.data.keys()
    }
}
