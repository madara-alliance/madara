use crate::error::consumer::format_error_context;
use crate::error::ConsumptionError;
use omniqueue::Delivery;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, warn};

const QUEUE_DELIVERY_RETRY_DELAY: Duration = Duration::from_millis(500);

pub async fn ack_delivery_with_retry(delivery: Delivery) -> Result<(), ConsumptionError> {
    if let Err((first_error, delivery)) = delivery.ack().await {
        let first_error = format_error_context(&first_error);
        warn!(error = %first_error, "Failed to ACK message, retrying once");

        sleep(QUEUE_DELIVERY_RETRY_DELAY).await;

        if let Err((second_error, _delivery)) = delivery.ack().await {
            let second_error = format_error_context(&second_error);
            error!(
                first_error = %first_error,
                error = %second_error,
                "Failed to ACK message after retry"
            );
            return Err(ConsumptionError::FailedToAcknowledgeMessage(second_error));
        }
    }

    Ok(())
}

pub async fn nack_delivery_with_retry(delivery: Delivery) -> Result<(), ConsumptionError> {
    if let Err((first_error, delivery)) = delivery.nack().await {
        let first_error = format_error_context(&first_error);
        warn!(error = %first_error, "Failed to NACK message, retrying once");

        sleep(QUEUE_DELIVERY_RETRY_DELAY).await;

        if let Err((second_error, _delivery)) = delivery.nack().await {
            let second_error = format_error_context(&second_error);
            error!(
                first_error = %first_error,
                error = %second_error,
                "Failed to NACK message after retry"
            );
            return Err(ConsumptionError::FailedToNackMessage(second_error));
        }
    }

    Ok(())
}
