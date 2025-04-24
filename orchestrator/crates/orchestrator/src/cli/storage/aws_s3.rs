use clap::Args;

/// Parameters used to config AWS S3.
#[derive(Debug, Clone, Args)]
#[group()] // Note: we are not using bucket_name in requires_all because it has a default value.
pub struct AWSS3CliArgs {
    /// Use the AWS s3 client
    #[arg(long)]
    pub aws_s3: bool,

    // REVIEW: 8 : Given in the `impl TryFrom<RunCmd> for StorageArgs {` we are using `aws_prefix`
    // A minor thing would be if someone defines aws_prefix as madara-orchestrator,
    // we'll have the name as madara-orchestrator-madara-orchestrator-bucket xD
    /// The name of the S3 bucket.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_S3_BUCKET_NAME", long, default_value = Some("madara-orchestrator-bucket"))]
    pub bucket_name: Option<String>,

    /// The S3 Bucket Location Constraint.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_BUCKET_LOCATION_CONSTRAINT", long)]
    pub bucket_location_constraint: Option<String>,
}
