use crate::error::job::JobError;
use crate::error::other::OtherError;
use std::str::FromStr;

pub fn parse_string<T: FromStr>(value: &str) -> Result<T, JobError>
where
    T::Err: std::fmt::Display,
{
    value.parse::<T>().map_err(|e| JobError::Other(OtherError::from(format!("Could not parse string: {e}"))))
}
