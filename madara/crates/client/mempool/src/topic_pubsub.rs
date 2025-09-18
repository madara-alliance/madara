use dashmap::{mapref::entry::Entry, DashMap};
use std::hash::Hash;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct TopicWatchPubsub<K: Hash + Eq + Clone, V: Clone>(Arc<DashMap<K, tokio::sync::watch::Sender<V>>>);

impl<K: Hash + Eq + Clone, V: Clone> TopicWatchPubsub<K, V> {
    pub fn publish(&self, key: &K, value: V) {
        if let dashmap::Entry::Occupied(entry) = self.0.entry(key.clone()) {
            if entry.get().send(value).is_err() {
                // Receiver closed (leak?)
                entry.remove();
            }
        }
    }

    pub fn watch<E>(
        &self,
        key: K,
        get_initial_state: impl FnOnce() -> Result<V, E>,
    ) -> Result<TopicWatchReceiver<K, V>, E> {
        match self.0.entry(key.clone()) {
            dashmap::Entry::Occupied(entry) => {
                Ok(TopicWatchReceiver { receiver: Some(entry.get().subscribe()), key, pubsub: self.clone() })
            }
            dashmap::Entry::Vacant(entry) => {
                let (sender, receiver) = tokio::sync::watch::channel(get_initial_state()?);
                entry.insert(sender);
                Ok(TopicWatchReceiver { receiver: Some(receiver), key, pubsub: self.clone() })
            }
        }
    }
}

pub struct TopicWatchReceiver<K: Hash + Eq + Clone, V: Clone> {
    receiver: Option<tokio::sync::watch::Receiver<V>>,
    key: K,
    pubsub: TopicWatchPubsub<K, V>,
}

impl<K: Hash + Eq + Clone, V: Clone> TopicWatchReceiver<K, V> {
    pub fn borrow(&self) -> impl Deref<Target = V> + '_ {
        self.receiver.as_ref().expect("Value already dropped").borrow()
    }
    pub fn borrow_and_update(&mut self) -> impl Deref<Target = V> + '_ {
        self.receiver.as_mut().expect("Value already dropped").borrow_and_update()
    }
    /// Marks the newest value as seen when returning.
    pub async fn changed(&mut self) {
        self.receiver.as_mut().expect("Value already dropped").changed().await.expect("Sender was dropped")
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Drop for TopicWatchReceiver<K, V> {
    fn drop(&mut self) {
        self.receiver.take(); // Drop the receiver first
        if let Entry::Occupied(entry) = self.pubsub.0.entry(self.key.clone()) {
            // The channel is now closed if the receiver we dropped was the last subscription to this key.
            if entry.get().is_closed() {
                entry.remove();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn basic_publish_subscribe() {
        let pubsub = TopicWatchPubsub::default();
        let mut rx = pubsub.watch("test", || Ok::<_, ()>(42)).unwrap();

        assert_eq!(*rx.borrow(), 42);
        pubsub.publish(&"test", 100);
        timeout(Duration::from_millis(10), rx.changed()).await.unwrap();
        assert_eq!(*rx.borrow(), 100);
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let pubsub = TopicWatchPubsub::default();
        let mut rx1 = pubsub.watch("topic", || Ok::<_, ()>(0)).unwrap();
        let mut rx2 = pubsub.watch("topic", || Ok::<_, ()>(999)).unwrap(); // Won't use initial value

        pubsub.publish(&"topic", 42);
        timeout(Duration::from_millis(10), rx1.changed()).await.unwrap();
        timeout(Duration::from_millis(10), rx2.changed()).await.unwrap();
        assert_eq!(*rx1.borrow(), 42);
        assert_eq!(*rx2.borrow(), 42);
    }

    #[tokio::test]
    async fn publish_to_nonexistent_topic() {
        let pubsub = TopicWatchPubsub::default();
        pubsub.publish(&"missing", 42); // Should not panic
        assert!(pubsub.0.is_empty());
    }

    #[tokio::test]
    async fn cleanup_on_drop() {
        let pubsub = TopicWatchPubsub::default();
        {
            let _rx = pubsub.watch("temp", || Ok::<_, ()>(1)).unwrap();
            assert_eq!(pubsub.0.len(), 1);
        } // rx dropped here
        assert!(pubsub.0.is_empty());
    }

    #[tokio::test]
    async fn borrow_and_update() {
        let pubsub = TopicWatchPubsub::default();
        let mut rx = pubsub.watch("test", || Ok::<_, ()>(1)).unwrap();

        pubsub.publish(&"test", 2);
        pubsub.publish(&"test", 3);

        let val = rx.borrow_and_update();
        assert_eq!(*val, 3);
    }
}
