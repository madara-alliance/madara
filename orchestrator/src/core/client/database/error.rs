use thiserror::Error;

#[derive(Error, Debug)]
pub enum DatabaseError {
    #[error("Bson Error: {0}")]
    BsonError(String),
    #[error("Mongo BSON Transform Error: {0}")]
    MongoBsonTransformError(#[from] mongodb::bson::ser::Error),
    #[error("Key is not found in the document: {0}")]
    KeyNotFound(String),
    #[error("Mongo Error: {0}")]
    MongoError(#[from] mongodb::error::Error),
    #[error("Item already exists: {0}")]
    ItemAlreadyExists(String),
    #[error("No update found: {0}")]
    NoUpdateFound(String),
    #[error("Update failed: {0}")]
    UpdateFailed(String),
    #[error("Failed to serialize document: {0}")]
    FailedToSerializeDocument(String),
    #[error("Failed to insert document: {0}")]
    InsertFailed(String),
}
