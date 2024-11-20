use crate::queue::sqs::AWSSQSValidatedArgs;

pub mod aws_sqs;

#[derive(Clone, Debug)]
pub enum QueueValidatedArgs {
    AWSSQS(AWSSQSValidatedArgs),
}
