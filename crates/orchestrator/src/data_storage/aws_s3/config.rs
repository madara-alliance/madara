use utils::env_utils::get_env_var_or_panic;

use crate::data_storage::DataStorageConfig;

/// Represents AWS S3 config struct with all the necessary variables.
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
