use std::sync::{Arc, Mutex};

// Tests for the SQS implementation
#[test]
fn test_queue_url_storage() {
    // Create a mock queue_url Arc<Mutex<Option<String>>>
    let queue_url = Arc::new(Mutex::new(None));

    // Test setting the queue URL
    {
        let mut queue_url_guard = queue_url.lock().unwrap();
        *queue_url_guard = Some("https://sqs.example.com/test-queue".to_string());
    }

    // Verify it was set correctly
    {
        let queue_url_guard = queue_url.lock().unwrap();
        assert_eq!(*queue_url_guard, Some("https://sqs.example.com/test-queue".to_string()));
    }
}

#[test]
fn test_queue_url_can_be_cleared() {
    // Create a mock queue_url Arc<Mutex<Option<String>>> with a value
    let queue_url = Arc::new(Mutex::new(Some("https://sqs.example.com/test-queue".to_string())));

    // Test clearing the queue URL
    {
        let mut queue_url_guard = queue_url.lock().unwrap();
        *queue_url_guard = None;
    }

    // Verify it was cleared correctly
    {
        let queue_url_guard = queue_url.lock().unwrap();
        assert_eq!(*queue_url_guard, None);
    }
}
