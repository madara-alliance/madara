use crate::database::mongodb::config::MongoDbConfig;
use crate::database::Database;
use crate::jobs::types::{JobItem, JobStatus, JobType};
use async_trait::async_trait;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use mongodb::bson::Document;
use mongodb::options::UpdateOptions;
use mongodb::{
    bson::doc,
    options::{ClientOptions, ServerApi, ServerApiVersion},
    Client, Collection,
};
use std::collections::HashMap;
use uuid::Uuid;

pub mod config;

pub struct MongoDb {
    client: Client,
}

impl MongoDb {
    pub async fn new(config: MongoDbConfig) -> Self {
        let mut client_options = ClientOptions::parse(config.url).await.expect("Failed to parse MongoDB Url");
        // Set the server_api field of the client_options object to set the version of the Stable API on the client
        let server_api = ServerApi::builder().version(ServerApiVersion::V1).build();
        client_options.server_api = Some(server_api);
        // Get a handle to the cluster
        let client = Client::with_options(client_options).expect("Failed to create MongoDB client");
        // Ping the server to see if you can connect to the cluster
        client.database("admin").run_command(doc! {"ping": 1}, None).await.expect("Failed to ping MongoDB deployment");
        println!("Pinged your deployment. You successfully connected to MongoDB!");

        MongoDb { client }
    }

    fn get_job_collection(&self) -> Collection<JobItem> {
        self.client.database("orchestrator").collection("jobs")
    }

    /// Updates the job in the database optimistically. This means that the job is updated only if the
    /// version of the job in the database is the same as the version of the job passed in. If the version
    /// is different, the update fails.
    async fn update_job_optimistically(&self, current_job: &JobItem, update: Document) -> Result<()> {
        let filter = doc! {
            "id": current_job.id,
            "version": current_job.version,
        };
        let options = UpdateOptions::builder().upsert(false).build();
        let result = self.get_job_collection().update_one(filter, update, options).await?;
        if result.modified_count == 0 {
            return Err(eyre!("Failed to update job. Job version is likely outdated"));
        }
        Ok(())
    }
}

#[async_trait]
impl Database for MongoDb {
    async fn create_job(&self, job: JobItem) -> Result<JobItem> {
        self.get_job_collection().insert_one(&job, None).await?;
        Ok(job)
    }

    async fn get_job_by_id(&self, id: Uuid) -> Result<Option<JobItem>> {
        let filter = doc! {
            "id":  id
        };
        Ok(self.get_job_collection().find_one(filter, None).await?)
    }

    async fn get_job_by_internal_id_and_type(&self, internal_id: &str, job_type: &JobType) -> Result<Option<JobItem>> {
        let filter = doc! {
            "internal_id": internal_id,
            "job_type": mongodb::bson::to_bson(&job_type)?,
        };
        Ok(self.get_job_collection().find_one(filter, None).await?)
    }

    async fn update_job_status(&self, job: &JobItem, new_status: JobStatus) -> Result<()> {
        let update = doc! {
            "$set": {
                "status": mongodb::bson::to_bson(&new_status)?,
            }
        };
        self.update_job_optimistically(job, update).await?;
        Ok(())
    }

    async fn update_external_id_and_status_and_metadata(
        &self,
        job: &JobItem,
        external_id: String,
        new_status: JobStatus,
        metadata: HashMap<String, String>,
    ) -> Result<()> {
        let update = doc! {
            "$set": {
                "status": mongodb::bson::to_bson(&new_status)?,
                "external_id": external_id,
                "metadata":  mongodb::bson::to_document(&metadata)?
            }
        };
        self.update_job_optimistically(job, update).await?;
        Ok(())
    }

    async fn update_metadata(&self, job: &JobItem, metadata: HashMap<String, String>) -> Result<()> {
        let update = doc! {
            "$set": {
                "metadata":  mongodb::bson::to_document(&metadata)?
            }
        };
        self.update_job_optimistically(job, update).await?;
        Ok(())
    }
}
