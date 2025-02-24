use crate::alerts::aws_sns::AWSSNSValidatedArgs;

pub mod aws_sns;

#[derive(Clone, Debug)]
pub enum AlertValidatedArgs {
    AWSSNS(AWSSNSValidatedArgs),
}
