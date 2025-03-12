
pub mod aws_sns;

#[derive(Clone, Debug)]
pub enum AlertValidatedArgs {
    AWSSNS(aws_sns::AWSSNSValidatedArgs),
}
