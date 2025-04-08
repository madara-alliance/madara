use crate::core::cloud::CloudProvider;
use crate::setup::creator::ResourceType;
use std::any::Any;
use std::sync::Arc;

// ResourceWrapper to type-erase the specific resource types
pub struct ResourceWrapper {
    resource: Box<dyn Any + Send + Sync>,
    resource_type: ResourceType,
    cloud_provider: Arc<CloudProvider>,
}

impl ResourceWrapper {
    pub fn new<R>(cloud_provider: Arc<CloudProvider>, resource: R, resource_type: ResourceType) -> Self
    where
        R: Any + Send + Sync,
    {
        ResourceWrapper { cloud_provider, resource: Box::new(resource), resource_type }
    }

    pub fn get_type(&self) -> &ResourceType {
        &self.resource_type
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.resource.downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.resource.downcast_mut::<T>()
    }
}
