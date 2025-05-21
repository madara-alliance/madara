use clap::Args;

/// Parameters used to config AWS S3.
#[derive(Debug, Clone, Args)]
#[group()] // Note: we are not using bucket_name in requires_all because it has a default value.
pub struct AWSS3CliArgs {
    /// Use the AWS s3 client
    #[arg(long)]
    pub aws_s3: bool,

    /// The ARN / Name of the S3 bucket.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_S3_BUCKET_IDENTIFIER", long, default_value = Some("mo-bucket"))]
    pub bucket_identifier: Option<String>,
}
