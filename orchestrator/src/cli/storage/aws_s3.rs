use clap::Args;

/// Default number of days before S3 deletes tagged objects
pub const DEFAULT_STORAGE_EXPIRATION_DAYS: i32 = 14;

/// Parameters used to config AWS S3.
#[derive(Debug, Clone, Args)]
#[group()] // Note: we are not using bucket_name in requires_all because it has a default value.
pub struct AWSS3CliArgs {
    /// Use the AWS s3 client
    #[arg(long)]
    pub aws_s3: bool,

    /// The ARN / Name of the S3 bucket.
    /// ARN: arn:aws:s3:::name
    /// We don't need to provide the region and accountID in s3
    /// because s3 is unique globally.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_S3_BUCKET_IDENTIFIER", long, default_value = Some("mo-bucket"))]
    pub bucket_identifier: Option<String>,

    /// Number of days before S3 automatically deletes objects tagged for expiration.
    /// Different deployments may need different retention periods (e.g., 7 days for dev, 30 days for production).
    #[arg(env = "MADARA_ORCHESTRATOR_STORAGE_EXPIRATION_DAYS", long, default_value_t = DEFAULT_STORAGE_EXPIRATION_DAYS)]
    pub storage_expiration_days: i32,
}
