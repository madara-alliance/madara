use opentelemetry::KeyValue;
use std::env;

/// Container-aware metric extensions for multi-instance deployments
pub struct MetricLabels;

impl MetricLabels {
    /// Get container-specific labels from environment variables
    pub fn get_container_labels() -> Vec<KeyValue> {
        let mut labels = vec![];
        
        // Container ID or hostname
        if let Ok(container_id) = env::var("CONTAINER_ID") {
            labels.push(KeyValue::new("container_id", container_id));
        } else if let Ok(hostname) = env::var("HOSTNAME") {
            labels.push(KeyValue::new("container_id", hostname));
        }
        
        // Instance ID for logical grouping
        if let Ok(instance_id) = env::var("INSTANCE_ID") {
            labels.push(KeyValue::new("instance_id", instance_id));
        }
        
        // Pod name for Kubernetes deployments
        if let Ok(pod_name) = env::var("POD_NAME") {
            labels.push(KeyValue::new("pod_name", pod_name));
        }
        
        // Node name for multi-node deployments
        if let Ok(node_name) = env::var("NODE_NAME") {
            labels.push(KeyValue::new("node_name", node_name));
        }
        
        // Deployment environment
        if let Ok(env_name) = env::var("DEPLOYMENT_ENV") {
            labels.push(KeyValue::new("environment", env_name));
        }
        
        labels
    }
    
    /// Create job-specific labels with container context
    pub fn job_labels(job_id: &str, job_type: &str, block_number: Option<u64>) -> Vec<KeyValue> {
        let mut labels = Self::get_container_labels();
        
        labels.push(KeyValue::new("job_id", job_id.to_string()));
        labels.push(KeyValue::new("job_type", job_type.to_string()));
        
        if let Some(block) = block_number {
            labels.push(KeyValue::new("block_number", block.to_string()));
        }
        
        labels
    }
    
    /// Create operation labels with container context
    pub fn operation_labels(operation_type: &str, operation_info: Option<&str>) -> Vec<KeyValue> {
        let mut labels = Self::get_container_labels();
        
        labels.push(KeyValue::new("operation_type", operation_type.to_string()));
        
        if let Some(info) = operation_info {
            labels.push(KeyValue::new("operation_info", info.to_string()));
        }
        
        labels
    }
}

/// Extension trait for adding container labels to metrics
pub trait ContainerMetrics {
    fn record_with_container(&self, value: f64, labels: &[KeyValue]);
    fn add_with_container(&self, value: f64, labels: &[KeyValue]);
}

/// Macro to simplify metric recording with container labels
#[macro_export]
macro_rules! record_metric_with_container {
    ($metric:expr, $value:expr, $($label:expr),*) => {
        {
            let mut labels = $crate::utils::metrics_extensions::MetricLabels::get_container_labels();
            $(
                labels.push($label);
            )*
            $metric.record($value, &labels);
        }
    };
}

#[macro_export]
macro_rules! add_metric_with_container {
    ($metric:expr, $value:expr, $($label:expr),*) => {
        {
            let mut labels = $crate::utils::metrics_extensions::MetricLabels::get_container_labels();
            $(
                labels.push($label);
            )*
            $metric.add($value, &labels);
        }
    };
}