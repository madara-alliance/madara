use utils::env_utils::get_env_var_or_panic;

use crate::data_storage::DataStorageConfig;

/// Represents the type of the config which one wants to pass to create the client
#[derive(Clone)]
pub enum AWSS3ConfigType {
    WithEndpoint(S3LocalStackConfig),
    WithoutEndpoint(AWSS3Config),
}

/// Represents AWS S3 config struct with all the necessary variables.
#[derive(Clone)]
pub struct AWSS3Config {
    /// AWS ACCESS KEY ID
    pub s3_key_id: String,
    /// AWS ACCESS KEY SECRET
    pub s3_key_secret: String,
    /// S3 Bucket Name
    pub s3_bucket_name: String,
    /// S3 Bucket region
    pub s3_bucket_region: String,
}

/// Represents AWS S3 config struct with all the necessary variables.
#[derive(Clone)]
pub struct S3LocalStackConfig {
    /// AWS ACCESS KEY ID
    pub s3_key_id: String,
    /// AWS ACCESS KEY SECRET
    pub s3_key_secret: String,
    /// S3 Bucket Name
    pub s3_bucket_name: String,
    /// S3 Bucket region
    pub s3_bucket_region: String,
    /// Endpoint url
    pub endpoint_url: String,
}

/// Implementation of `DataStorageConfig` for `AWSS3Config`
impl DataStorageConfig for AWSS3Config {
    /// To return the config struct by creating it from the environment variables.
    fn new_from_env() -> Self {
        Self {
            s3_key_id: get_env_var_or_panic("AWS_ACCESS_KEY_ID"),
            s3_key_secret: get_env_var_or_panic("AWS_SECRET_ACCESS_KEY"),
            s3_bucket_name: get_env_var_or_panic("AWS_S3_BUCKET_NAME"),
            s3_bucket_region: get_env_var_or_panic("AWS_S3_BUCKET_REGION"),
        }
    }
}

/// Implementation of `DataStorageConfig` for `S3LocalStackConfig`
impl DataStorageConfig for S3LocalStackConfig {
    /// To return the config struct by creating it from the environment variables.
    fn new_from_env() -> Self {
        Self {
            s3_key_id: get_env_var_or_panic("AWS_ACCESS_KEY_ID"),
            s3_key_secret: get_env_var_or_panic("AWS_SECRET_ACCESS_KEY"),
            s3_bucket_name: get_env_var_or_panic("AWS_S3_BUCKET_NAME"),
            s3_bucket_region: get_env_var_or_panic("AWS_S3_BUCKET_REGION"),
            endpoint_url: get_env_var_or_panic("AWS_ENDPOINT_URL"),
        }
    }
}
