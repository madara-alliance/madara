use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::delete_object::DeleteObjectError;
use aws_sdk_s3::operation::get_object::GetObjectError;
use aws_sdk_s3::operation::list_buckets::ListBucketsError;
use aws_sdk_s3::operation::put_object::PutObjectError;
use aws_sdk_sqs::operation::set_queue_attributes::SetQueueAttributesError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Invalid cloud region error: {0}")]
    InvalidRegionError(String),
    /// AWS SDK error
    #[error("Failed to list buckets: {0}")]
    ListBucketsError(#[from] SdkError<ListBucketsError>),
    #[error("Failed to put object : {0}")]
    UnableToPutObject(#[from] SdkError<PutObjectError>),
    #[error("Unable to delete object : {0}")]
    DeleteObjectError(#[from] SdkError<DeleteObjectError>),
    /// AWS SQS error
    #[error(": {0}")]
    SetQueueAttributesError(#[from] SdkError<SetQueueAttributesError>),
    /// AWS S3 error
    #[error("Failed to get data from S3: {0}")]
    GetObjectError(#[from] SdkError<GetObjectError>),
    #[error("Failed to stream object: {0}")]
    ObjectStreamError(String),
    #[error("Invalid Bucket Name is given: {0}")]
    InvalidBucketName(String),
}
