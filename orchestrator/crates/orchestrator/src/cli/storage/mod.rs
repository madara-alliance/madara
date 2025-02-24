use crate::data_storage::aws_s3::AWSS3ValidatedArgs;

pub mod aws_s3;

#[derive(Clone, Debug)]
pub enum StorageValidatedArgs {
    AWSS3(AWSS3ValidatedArgs),
}
