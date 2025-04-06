pub mod aws_s3;

#[derive(Debug, Clone)]
pub struct AWSS3ValidatedArgs {
    pub bucket_name: String,
}

#[derive(Clone, Debug)]
pub enum StorageValidatedArgs {
    AWSS3(AWSS3ValidatedArgs),
}
