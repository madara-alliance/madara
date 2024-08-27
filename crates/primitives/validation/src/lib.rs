use dp_rayon_pool::RayonPool;

pub trait ValidationContext: Clone + Send + Sync + 'static {}

#[async_trait::async_trait]
pub trait Validate: Send + Sync + Sized + 'static {
    type Output: Send + 'static;
    type ValidationError: Send;
    type Context: ValidationContext;

    fn validate(self, context: &Self::Context) -> Result<Self::Output, Self::ValidationError>;

    async fn spawn_validate(
        self,
        pool: &RayonPool,
        context: Self::Context,
    ) -> Result<Self::Output, Self::ValidationError> {
        pool.spawn_rayon_task(move || self.validate(&context)).await
    }
}
