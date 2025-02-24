use std::str::FromStr;

use crate::jobs::{JobError, OtherError};

pub fn parse_string<T: FromStr>(value: &str) -> Result<T, JobError>
where
    T::Err: std::fmt::Display,
{
    value.parse::<T>().map_err(|e| JobError::Other(OtherError::from(format!("Could not parse string: {e}"))))
}
