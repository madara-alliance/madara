use dashmap::{mapref::entry::Entry, DashMap};
use smallvec::SmallVec;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct TopicWatchPubsub<K: Hash + Eq + Clone, V: Clone>(Arc<DashMap<K, tokio::sync::watch::Sender<V>>>);

impl<K: Hash + Eq + Clone, V: Clone> TopicWatchPubsub<K, V> {
    pub fn publish(&self, key: &K, value: V) {
        if let dashmap::Entry::Occupied(mut entry) = self.0.entry(key.clone()) {
            if entry.get().send(value).is_err() {
                // Closed (leak?)
                entry.remove()
            }
        }
    }

    pub fn subscribe<E>(
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
    pub fn borrow(&self) -> impl Deref<Target = V> {
        self.receiver.as_ref().expect("Value already dropped").borrow()
    }
    pub fn borrow_and_update(&mut self) -> impl Deref<Target = V> {
        self.receiver.as_mut().expect("Value already dropped").borrow_and_update()
    }
    pub async fn recv(&mut self) -> V {
        self.receiver.as_mut().expect("Value already dropped").changed()
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Drop for TopicWatchReceiver<K, V> {
    fn drop(&mut self) {
        self.receiver.take(); // Drop the receiver first
        if let Entry::Occupied(mut entry) = self.pubsub.0.entry(self.key.clone()) {
            // The channel is now closed if the receiver we dropped was the last subscription to this key.
            if entry.get().is_closed() {
                entry.remove();
            }
        }
    }
}
