use parking_lot::Mutex;
use std::collections::HashMap;

use crate::trace_lock;

/// Wrapper for `Mutex<Vec<T>>` with simplifies the API and prevents deadlocks
pub struct SyncVec<T>(Mutex<Vec<T>>);

#[allow(dead_code)]
impl<T> SyncVec<T> {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    pub fn len(&self) -> usize {
        trace_lock!(self.0).len()
    }

    pub fn push(&self, item: T) {
        trace_lock!(self.0).push(item);
    }

    pub fn push_if_missing(&self, item: T, mut exists: impl FnMut(&T) -> bool) -> bool {
        let mut vec = trace_lock!(self.0);
        if vec.iter().any(&mut exists) {
            return false;
        }
        vec.push(item);
        true
    }

    pub fn any(&self, f: impl FnMut(&T) -> bool) -> bool {
        trace_lock!(self.0).iter().any(f)
    }

    pub fn for_each<F>(&self, f: F)
    where
        F: FnMut(&mut T),
    {
        trace_lock!(self.0).iter_mut().for_each(f);
    }

    pub fn retain<F>(&self, f: F)
    where
        F: FnMut(&T) -> bool,
    {
        trace_lock!(self.0).retain(f);
    }

    pub fn clear(&self) {
        trace_lock!(self.0).clear();
    }
}

impl<T> From<Vec<T>> for SyncVec<T> {
    fn from(value: Vec<T>) -> Self {
        Self(Mutex::new(value))
    }
}

/// Wrapper for `Mutex<HashMap<K, V>>` with simplified API and prevents deadlocks
pub struct SyncHashMap<K, V>(Mutex<HashMap<K, V>>)
where
    K: Eq + std::hash::Hash;

#[allow(dead_code)]
impl<K, V> SyncHashMap<K, V>
where
    K: Eq + std::hash::Hash + Clone,
    V: Clone,
{
    pub fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }

    pub fn len(&self) -> usize {
        trace_lock!(self.0).len()
    }

    pub fn get(&self, key: &K) -> Option<V> {
        trace_lock!(self.0).get(key).cloned()
    }

    pub fn values(&self) -> Vec<V> {
        trace_lock!(self.0).values().cloned().collect()
    }

    pub fn iter(&self) -> Vec<(K, V)> {
        trace_lock!(self.0)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn upsert(&self, key: K, value: V) {
        trace_lock!(self.0).insert(key, value);
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        trace_lock!(self.0).remove(key)
    }

    pub fn clear(&self) {
        trace_lock!(self.0).clear();
    }
}

impl<K, V> From<HashMap<K, V>> for SyncHashMap<K, V>
where
    K: Eq + std::hash::Hash,
{
    fn from(value: HashMap<K, V>) -> Self {
        Self(Mutex::new(value))
    }
}
