mod mongo;
mod r#trait;

pub use mongo::MongoJobRepository;
pub use r#trait::JobRepository;

#[cfg(test)]
pub use r#trait::MockJobRepository;
