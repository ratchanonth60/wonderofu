//! Small memoization primitives.

use std::{
    collections::{HashMap, hash_map::Entry},
    hash::Hash,
};

/// A simple hash map backed memoization cache.
#[derive(Debug, Clone, Default)]
pub struct MemoCache<K, V> {
    cache: HashMap<K, V>,
}

impl<K, V> MemoCache<K, V>
where
    K: Eq + Hash,
{
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Returns the cached value for a key, computing it when missing.
    pub fn get_or_insert_with(&mut self, key: K, f: impl FnOnce() -> V) -> &V {
        match self.cache.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(f()),
        }
    }

    /// Returns the cached value for a key, computing it fallibly when missing.
    pub fn get_or_try_insert_with<E>(
        &mut self,
        key: K,
        f: impl FnOnce() -> Result<V, E>,
    ) -> Result<&V, E> {
        match self.cache.entry(key) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => Ok(entry.insert(f()?)),
        }
    }

    /// Clears all memoized values.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Returns the number of cached entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::MemoCache;

    #[test]
    fn memoizes_values_once_per_key() {
        let mut cache = MemoCache::new();
        let calls = Cell::new(0);

        let first = *cache.get_or_insert_with("key", || {
            calls.set(calls.get() + 1);
            41
        });
        let second = *cache.get_or_insert_with("key", || {
            calls.set(calls.get() + 1);
            99
        });

        assert_eq!(first, 41);
        assert_eq!(second, 41);
        assert_eq!(calls.get(), 1);
    }
}
