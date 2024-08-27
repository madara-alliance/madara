use dp_rayon_pool::RayonPool;

#[async_trait::async_trait]
pub trait Validate: Send + Sync + Sized + 'static {
    type Output: Send + 'static;
    type Context: Clone + Send + Sync + 'static;
    type ValidationError: Send;

    fn validate(self, context: &Self::Context) -> Result<Self::Output, Self::ValidationError>;

    async fn spawn_validate(
        self,
        pool: &RayonPool,
        context: Self::Context,
    ) -> Result<Self::Output, Self::ValidationError> {
        pool.spawn_rayon_task(move || self.validate(&context)).await
    }
}
