use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_s3::{
    config::Region,
    error::SdkError,
    operation::{
        create_bucket::CreateBucketError, delete_bucket::DeleteBucketError, delete_object::DeleteObjectError,
        get_object::GetObjectError, head_bucket::HeadBucketError, head_object::HeadObjectError,
        list_objects_v2::ListObjectsV2Error, put_object::PutObjectError,
    },
    presigning::PresigningConfig,
    types::{BucketLocationConstraint, CreateBucketConfiguration, Object},
    Client as S3Client,
};
use bytes::Bytes;
use chrono;
use std::{collections::HashMap, path::Path, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt};
use tokio_util::io::ReaderStream;

use crate::{
    core::client::storage::{ObjectMetadata, StorageClient},
    error::{Error, Result},
};
use crate::core::cloud::CloudProvider;

/// AWS S3 client implementation
pub struct S3StorageClient {
    /// S3 client
    client: Option<S3Client>,
    /// AWS region
    region: String,
}

impl S3StorageClient {
    /// Create a new S3 client
    pub fn new(provider: &CloudProvider) -> Self {
        Self { client: None, region: region.to_string() }
    }

    /// Get a reference to the S3 client
    fn get_client(&self) -> Result<&S3Client> {
        self.client.as_ref().ok_or_else(|| Error::ResourceError("S3 client not initialized".to_string()))
    }

    /// Convert S3 Object to ObjectMetadata
    fn convert_object(&self, obj: &Object) -> ObjectMetadata {
        let last_modified = if let Some(dt) = obj.last_modified() {
            let timestamp = dt.as_secs_f64();
            Some(chrono::DateTime::from_timestamp(timestamp as i64, 0).unwrap())
        } else {
            None
        };

        ObjectMetadata {
            key: obj.key().unwrap_or_default().to_string(),
            size: obj.size() as u64,
            last_modified,
            etag: obj.e_tag().map(|s| s.to_string()),
            content_type: None,       // Not available in list objects response
            metadata: HashMap::new(), // Not available in list objects response
        }
    }
}

#[async_trait]
impl StorageClient for S3StorageClient {
    async fn init(&self) -> Result<()> {
        let mut this = self.to_owned();

        let config =
            aws_config::defaults(BehaviorVersion::latest()).region(Region::new(this.region.clone())).load().await;

        let client = S3Client::new(&config);
        this.client = Some(client);

        Ok(())
    }

    async fn bucket_exists(&self, bucket: &str) -> Result<bool> {
        let client = self.get_client()?;

        let result = client.head_bucket().bucket(bucket).send().await;

        match result {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    HeadBucketError::NotFound(_) => Ok(false),
                    _ => Err(Error::ResourceError(format!("Failed to check if bucket exists: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to check if bucket exists: {:?}", err))),
        }
    }

    async fn create_bucket(&self, bucket: &str) -> Result<()> {
        let client = self.get_client()?;

        let constraint = BucketLocationConstraint::from(self.region.as_str());
        let cfg = CreateBucketConfiguration::builder().location_constraint(constraint).build();

        let result = client.create_bucket().bucket(bucket).create_bucket_configuration(cfg).send().await;

        match result {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    CreateBucketError::BucketAlreadyExists(_) => Ok(()),
                    CreateBucketError::BucketAlreadyOwnedByYou(_) => Ok(()),
                    _ => Err(Error::ResourceError(format!("Failed to create bucket: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to create bucket: {:?}", err))),
        }
    }

    async fn delete_bucket(&self, bucket: &str) -> Result<()> {
        let client = self.get_client()?;

        let result = client.delete_bucket().bucket(bucket).send().await;

        match result {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    DeleteBucketError::NoSuchBucket(_) => Ok(()),
                    _ => Err(Error::ResourceError(format!("Failed to delete bucket: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to delete bucket: {:?}", err))),
        }
    }

    async fn list_objects(&self, bucket: &str, prefix: Option<&str>) -> Result<Vec<ObjectMetadata>> {
        let client = self.get_client()?;

        let mut request = client.list_objects_v2().bucket(bucket);

        if let Some(prefix) = prefix {
            request = request.prefix(prefix);
        }

        let result = request.send().await;

        match result {
            Ok(output) => {
                let objects = output.contents().unwrap_or_default();
                let metadata = objects.iter().map(|obj| self.convert_object(obj)).collect();
                Ok(metadata)
            }
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    ListObjectsV2Error::NoSuchBucket(_) => {
                        Err(Error::ResourceError("Bucket does not exist".to_string()))
                    }
                    _ => Err(Error::ResourceError(format!("Failed to list objects: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to list objects: {:?}", err))),
        }
    }

    async fn put_object(&self, bucket: &str, key: &str, data: Bytes, content_type: Option<&str>) -> Result<()> {
        let client = self.get_client()?;

        let mut request = client.put_object().bucket(bucket).key(key).body(data.into());

        if let Some(content_type) = content_type {
            request = request.content_type(content_type);
        }

        let result = request.send().await;

        match result {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                Err(Error::ResourceError(format!("Failed to put object: {:?}", err)))
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to put object: {:?}", err))),
        }
    }

    async fn upload_file(&self, bucket: &str, key: &str, file_path: &Path) -> Result<()> {
        let client = self.get_client()?;

        let file = File::open(file_path).await.map_err(|e| Error::IoError(e))?;

        let body = ReaderStream::new(file);
        let body = aws_sdk_s3::primitives::ByteStream::new(body.into());

        // Try to determine content type from file extension
        let content_type = mime_guess::from_path(file_path).first_raw().unwrap_or("application/octet-stream");

        let result = client.put_object().bucket(bucket).key(key).body(body).content_type(content_type).send().await;

        match result {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                Err(Error::ResourceError(format!("Failed to upload file: {:?}", err)))
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to upload file: {:?}", err))),
        }
    }

    async fn get_object(&self, bucket: &str, key: &str) -> Result<Bytes> {
        let client = self.get_client()?;

        let result = client.get_object().bucket(bucket).key(key).send().await;

        match result {
            Ok(output) => {
                let data = output
                    .body
                    .collect()
                    .await
                    .map_err(|e| Error::ResourceError(format!("Failed to read object data: {:?}", e)))?;

                Ok(data.into_bytes())
            }
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    GetObjectError::NoSuchKey(_) => Err(Error::ResourceError("Object does not exist".to_string())),
                    _ => Err(Error::ResourceError(format!("Failed to get object: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to get object: {:?}", err))),
        }
    }

    async fn download_file(&self, bucket: &str, key: &str, file_path: &Path) -> Result<()> {
        let data = self.get_object(bucket, key).await?;

        let mut file = File::create(file_path).await.map_err(|e| Error::IoError(e))?;

        file.write_all(&data).await.map_err(|e| Error::IoError(e))?;

        Ok(())
    }

    async fn object_exists(&self, bucket: &str, key: &str) -> Result<bool> {
        let client = self.get_client()?;

        let result = client.head_object().bucket(bucket).key(key).send().await;

        match result {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    HeadObjectError::NotFound(_) => Ok(false),
                    _ => Err(Error::ResourceError(format!("Failed to check if object exists: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to check if object exists: {:?}", err))),
        }
    }

    async fn get_object_metadata(&self, bucket: &str, key: &str) -> Result<ObjectMetadata> {
        let client = self.get_client()?;

        let result = client.head_object().bucket(bucket).key(key).send().await;

        match result {
            Ok(output) => {
                let last_modified = if let Some(dt) = output.last_modified() {
                    let timestamp = dt.as_secs_f64();
                    Some(chrono::DateTime::from_timestamp(timestamp as i64, 0).unwrap())
                } else {
                    None
                };

                let metadata = ObjectMetadata {
                    key: key.to_string(),
                    size: output.content_length() as u64,
                    last_modified,
                    etag: output.e_tag().map(|s| s.to_string()),
                    content_type: output.content_type().map(|s| s.to_string()),
                    metadata: output
                        .metadata()
                        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                        .unwrap_or_default(),
                };

                Ok(metadata)
            }
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                match err {
                    HeadObjectError::NotFound(_) => Err(Error::ResourceError("Object does not exist".to_string())),
                    _ => Err(Error::ResourceError(format!("Failed to get object metadata: {:?}", err))),
                }
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to get object metadata: {:?}", err))),
        }
    }

    async fn delete_object(&self, bucket: &str, key: &str) -> Result<()> {
        let client = self.get_client()?;

        let result = client.delete_object().bucket(bucket).key(key).send().await;

        match result {
            Ok(_) => Ok(()),
            Err(SdkError::ServiceError(err)) => {
                let err = err.err();
                Err(Error::ResourceError(format!("Failed to delete object: {:?}", err)))
            }
            Err(err) => Err(Error::ResourceError(format!("Failed to delete object: {:?}", err))),
        }
    }

    async fn generate_presigned_url(&self, bucket: &str, key: &str, expires_in: std::time::Duration) -> Result<String> {
        let client = self.get_client()?;

        let expires_in = std::time::SystemTime::now() + expires_in;

        let presigned_req = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .presigned(PresigningConfig::expires_at(expires_in).unwrap())
            .await
            .map_err(|e| Error::ResourceError(format!("Failed to generate presigned URL: {:?}", e)))?;

        Ok(presigned_req.uri().to_string())
    }
}
