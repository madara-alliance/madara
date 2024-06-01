use async_trait::async_trait;

pub mod proof_registration;
pub mod proving;
pub mod snos;
pub mod update_state;

#[async_trait]
pub trait Worker: Send + Sync {
    async fn run_worker(&self);
}
