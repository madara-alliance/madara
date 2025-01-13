use clap::Args;

/// Parameters used to config MongoDB.
#[derive(Debug, Clone, Args)]
#[group(requires_all = ["mongodb_connection_url"])]
pub struct MongoDBCliArgs {
    /// Use the MongoDB client
    #[arg(long)]
    pub mongodb: bool,

    /// The connection string to the MongoDB server.
    #[arg(env = "MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL", long, default_value = Some("mongodb://localhost:27017"))]
    pub mongodb_connection_url: Option<String>,

    /// The name of the database.
    #[arg(env = "MADARA_ORCHESTRATOR_DATABASE_NAME", long, default_value = Some("orchestrator"))]
    pub mongodb_database_name: Option<String>,
}
