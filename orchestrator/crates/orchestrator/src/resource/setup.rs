use crate::cli::provider::ProviderValidatedArgs;
use crate::cli::SetupCmd;
use crate::core::cloud::CloudProvider;
use crate::error::{OrchestratorError, OrchestratorResult};
use crate::resource::args::{AlertArgs, CronArgs, QueueArgs, StorageArgs};
use crate::resource::aws::s3::SSS;
use crate::resource::aws::sqs::SQS;
use crate::resource::Resource;
use async_trait::async_trait;
use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, instrument};

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub enum ResourceType {
    Queue,
    Storage,
    Cron,
    Notification,
}

impl ResourceType {
    pub fn from_str(resource_type: &str) -> Option<Self> {
        match resource_type.to_lowercase().as_str() {
            "queue" => Some(ResourceType::Queue),
            "storage" => Some(ResourceType::Storage),
            "cron" => Some(ResourceType::Cron),
            "notification" => Some(ResourceType::Notification),
            _ => None,
        }
    }
}

// ResourceWrapper to type-erase the specific resource types
pub struct ResourceWrapper {
    resource: Box<dyn Any + Send + Sync>,
    resource_type: ResourceType,
    cloud_provider: Arc<CloudProvider>,
}

impl ResourceWrapper {
    pub fn new<R>(cloud_provider: Arc<CloudProvider>, resource: R, resource_type: ResourceType) -> Self
    where
        R: Any + Send + Sync,
    {
        ResourceWrapper { cloud_provider, resource: Box::new(resource), resource_type }
    }

    pub fn get_type(&self) -> &ResourceType {
        &self.resource_type
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.resource.downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.resource.downcast_mut::<T>()
    }
}

/// A trait for resource creation strategies to enable flexible resource instantiation
#[async_trait]
pub trait ResourceCreator: Send + Sync {
    async fn create_resource(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper>;
}

// S3 resource creator
pub struct S3ResourceCreator;

#[async_trait]
impl ResourceCreator for S3ResourceCreator {
    async fn create_resource(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper> {
        let s3 = SSS::new(cloud_provider.clone()).await?;
        Ok(ResourceWrapper::new(cloud_provider, s3, ResourceType::Storage))
    }
}

// SQS resource creator
pub struct SQSResourceCreator;

#[async_trait]
impl ResourceCreator for SQSResourceCreator {
    async fn create_resource(&self, cloud_provider: Arc<CloudProvider>) -> OrchestratorResult<ResourceWrapper> {
        let sqs = SQS::new(cloud_provider.clone()).await?;
        Ok(ResourceWrapper::new(cloud_provider, sqs, ResourceType::Queue))
    }
}

/// ResourceFactory is responsible for creating resources based on their type
pub struct ResourceFactory {
    creators: HashMap<ResourceType, Box<dyn ResourceCreator>>,
    cloud_provider: Arc<CloudProvider>,
    queue_params: QueueArgs,
    cron_params: CronArgs,
    storage_params: StorageArgs,
    alert_params: AlertArgs,
}

impl ResourceFactory {
    /// new_with_gcs - Create a new ResourceFactory with default resource creators for Orchestrator
    /// with GCS Cloud Provider
    pub fn new_with_gcs(
        cloud_provider: Arc<CloudProvider>,
        queue_params: QueueArgs,
        cron_params: CronArgs,
        storage_params: StorageArgs,
        alert_params: AlertArgs,
    ) -> Self {
        let mut creators = HashMap::new();
        ResourceFactory { creators, cloud_provider, queue_params, cron_params, storage_params, alert_params }
    }

    /// new_with_aws - Create a new ResourceFactory with default resource creators for Orchestrator
    /// with AWS Cloud Provider
    pub fn new_with_aws(
        cloud_provider: Arc<CloudProvider>,
        queue_params: QueueArgs,
        cron_params: CronArgs,
        storage_params: StorageArgs,
        alert_params: AlertArgs,
    ) -> Self {
        let mut creators = HashMap::new();
        creators.insert(ResourceType::Storage, Box::new(S3ResourceCreator) as Box<dyn ResourceCreator>);
        creators.insert(ResourceType::Queue, Box::new(SQSResourceCreator) as Box<dyn ResourceCreator>);

        ResourceFactory { creators, cloud_provider, queue_params, cron_params, storage_params, alert_params }
    }

    pub async fn setup_resource(&self) -> OrchestratorResult<()> {
        for (resource_type, creator) in self.creators.iter() {
            info!(" ⏳ Setting up resource: {:?}", resource_type);
            let mut resource = creator.create_resource(self.cloud_provider.clone()).await?;
            match resource_type {
                ResourceType::Storage => {
                    let rs = resource.downcast_mut::<SSS>().unwrap();
                    rs.setup(self.storage_params.clone()).await?;
                }
                ResourceType::Queue => {
                    let rs = resource.downcast_mut::<SQS>().unwrap();
                    rs.setup(self.queue_params.clone()).await?;
                }
                _ => {}
            }
            info!(" ✅ Resource setup completed: {:?}", resource_type);
        }
        Ok(())
    }

    /// Register a new resource creator
    pub fn register(&mut self, resource_type: ResourceType, creator: Box<dyn ResourceCreator>) {
        self.creators.insert(resource_type, creator);
    }

    /// Create a resource of the specified type
    pub async fn create_resource(
        &self,
        resource_type: ResourceType,
        cloud_provider: Arc<CloudProvider>,
    ) -> OrchestratorResult<ResourceWrapper> {
        match self.creators.get(&resource_type) {
            Some(creator) => creator.create_resource(cloud_provider).await,
            None => Err(OrchestratorError::ResourceError(format!(
                "No creator registered for resource type {:?}",
                resource_type
            ))),
        }
    }

    /// Create a resource from a string type
    pub async fn create_resource_from_str(
        &self,
        resource_type: &str,
        cloud_provider: Arc<CloudProvider>,
    ) -> OrchestratorResult<ResourceWrapper> {
        match ResourceType::from_str(resource_type) {
            Some(rt) => self.create_resource(rt, cloud_provider).await,
            None => Err(OrchestratorError::ResourceError(format!("Unknown resource type: {}", resource_type))),
        }
    }
}

/// Setup function that initializes all necessary resources
#[instrument]
pub async fn setup(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    let cloud_provider = setup_cloud_provider(&setup_cmd).await?;

    info!("Setting up resources for Orchestrator...");

    let queue_params = setup_cmd.validate_queue_params().map_err(|e| OrchestratorError::SetupCommandError(e))?;
    let storage_params = setup_cmd.validate_storage_params().map_err(|e| OrchestratorError::SetupCommandError(e))?;
    let alert_params = setup_cmd.validate_alert_params().map_err(|e| OrchestratorError::SetupCommandError(e))?;
    let cron_params = setup_cmd.validate_cron_params().map_err(|e| OrchestratorError::SetupCommandError(e))?;

    let resources = match cloud_provider.clone().get_provider_name().as_str() {
        "AWS" => ResourceFactory::new_with_aws(cloud_provider, queue_params, cron_params, storage_params, alert_params),
        a => Err(OrchestratorError::InvalidCloudProviderError(a.to_string()))?,
    };
    resources.setup_resource().await?;

    Ok(())
}

/// Set up the orchestrator with the provided configuration
pub async fn setup_cloud_provider(setup_cmd: &SetupCmd) -> OrchestratorResult<Arc<CloudProvider>> {
    let provider_params = setup_cmd.validate_provider_params().map_err(|e| OrchestratorError::SetupCommandError(e))?;

    info!("Initializing cloud provider...");
    let aws_config = match provider_params {
        ProviderValidatedArgs::AWS(aws_config) => aws_config,
    };

    // Create AWS SDK config
    let sdk_config = aws_config::from_env()
        .region(Region::new(aws_config.aws_region.clone()))
        .credentials_provider(Credentials::from_keys(
            &aws_config.aws_access_key_id,
            &aws_config.aws_secret_access_key,
            None,
        ))
        .load()
        .await;

    // Validate AWS access by attempting to list S3 buckets
    info!("Validating AWS credentials and permissions...");
    let s3_client = aws_sdk_s3::Client::new(&sdk_config);
    s3_client
        .list_buckets()
        .send()
        .await
        .map_err(|e| OrchestratorError::InvalidCloudProviderError(format!("Failed to validate AWS access: {}", e)))?;
    info!("AWS credentials validated successfully");

    let cloud_provider = Arc::new(CloudProvider::AWS(Box::new(sdk_config)));

    Ok(cloud_provider)
}

pub async fn setup_db() -> color_eyre::Result<()> {
    // We run the js script in the folder root:
    println!("Setting up database.");

    // Command::new("node").arg("migrate-mongo-config.js").output()?;

    println!("Database setup completed ✅");

    Ok(())
}
