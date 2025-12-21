mod mongo;
mod r#trait;

pub use mongo::MongoBatchRepository;
pub use r#trait::BatchRepository;

#[cfg(test)]
pub use r#trait::MockBatchRepository;
