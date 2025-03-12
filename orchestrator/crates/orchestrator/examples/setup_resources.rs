use aws_config::{Region, SdkConfig};
use clap::Parser;
use orchestrator::core::cloud::CloudProvider;
use orchestrator::error::OrchestratorResult;
use orchestrator::resource::aws::s3::{S3BucketSetupArgs, SSS};
use orchestrator::resource::aws::sqs::{SQSSetupArgs, SQS};
use orchestrator::resource::Resource;

/// CLI arguments for resource setup
#[derive(Parser, Debug)]
#[clap(author, version, about = "Setup AWS resources for Madara Orchestrator")]
struct Args {
    /// Prefix to use for all resources
    #[clap(short, long, env = "MADARA_ORCHESTRATOR_PREFIX")]
    prefix: String,

    /// S3 bucket name suffix
    #[clap(long, env = "MADARA_ORCHESTRATOR_AWS_S3_BUCKET_NAME", default_value = "bucket")]
    bucket_name: String,

    /// AWS region
    #[clap(long, env = "AWS_REGION", default_value = "us-west-1")]
    region: String,

    /// SQS queue suffix
    #[clap(long, env = "MADARA_ORCHESTRATOR_SQS_SUFFIX", default_value = "queue")]
    queue_suffix: String,

    /// SQS base URL
    #[clap(
        long,
        env = "MADARA_ORCHESTRATOR_SQS_BASE_QUEUE_URL",
        default_value = "https://sqs.us-west-1.amazonaws.com/"
    )]
    sqs_base_url: String,
}

#[tokio::main]
async fn main() -> OrchestratorResult<()> {
    // Parse CLI arguments
    let args = Args::parse();

    // Load AWS configuration with explicit region
    let config = aws_config::from_env().region(Region::new(args.region.clone())).load().await;

    let cloud_provider = CloudProvider::AWS(Box::new(config));

    println!("Using prefix: {}", args.prefix);
    println!("Using AWS region: {}", args.region);

    // Set up S3 bucket
    setup_s3_bucket(&cloud_provider, &args).await?;

    // Set up SQS queues
    setup_sqs_queue(&cloud_provider, "notifications", &args).await?;
    setup_sqs_queue(&cloud_provider, "jobs", &args).await?;

    println!("All resources have been set up successfully!");
    Ok(())
}

async fn setup_s3_bucket(cloud_provider: &CloudProvider, args: &Args) -> OrchestratorResult<()> {
    println!("Setting up S3 bucket...");

    // Create S3 resource
    let s3 = SSS::new(cloud_provider.clone()).await?;

    // Create the full bucket name with prefix
    let bucket_name = format!("{}-{}", args.prefix, args.bucket_name);
    println!("Creating bucket: {}", bucket_name);

    // Create setup args
    let setup_args = S3BucketSetupArgs { bucket_name, bucket_location_constraint: args.region.clone() };

    // Set up the bucket
    match s3.setup(setup_args).await {
        Ok(result) => {
            println!("S3 bucket '{}' created successfully!", result.name);
            Ok(())
        }
        Err(e) => {
            if e.to_string().contains("already exists") {
                println!("S3 bucket already exists, skipping creation.");
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

async fn setup_sqs_queue(cloud_provider: &CloudProvider, base_name: &str, args: &Args) -> OrchestratorResult<()> {
    println!("Setting up SQS queue for {}...", base_name);

    // Create SQS resource
    let sqs = SQS::new(cloud_provider.clone()).await?;

    // Create the full queue name with prefix and suffix
    let queue_name = format!("{}-{}-{}", args.prefix, base_name, args.queue_suffix);
    println!("Creating queue: {}", queue_name);

    // Get a full queue URL
    let queue_url = SQS::get_queue_url(&args.sqs_base_url, &queue_name);
    println!("Queue URL will be: {}", queue_url);

    // Create setup args
    let setup_args = SQSSetupArgs {
        queue_name,
        visibility_timeout: 30, // Default visibility timeout
    };

    // Set up the queue
    match sqs.setup(setup_args).await {
        Ok(result) => {
            println!("SQS queue '{}' created or confirmed!", result.queue_name);
            Ok(())
        }
        Err(e) => {
            println!("Error setting up SQS queue: {}", e);
            Err(e)
        }
    }
}
