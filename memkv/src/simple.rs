//! Simple BTreeMap-based implementation as a baseline.
//!
//! This uses a BTreeMap with byte vectors as keys. It's not as memory
//! efficient as a proper trie, but it's correct and gives us a working
//! baseline to compare against.

use std::collections::BTreeMap;
use std::ops::Bound;

/// A simple key-value store using BTreeMap.
pub struct SimpleKV<V> {
    map: BTreeMap<Vec<u8>, V>,
}

impl<V> SimpleKV<V> {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    /// Insert a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        self.map.insert(key.to_vec(), value)
    }

    /// Get a reference to the value for a key.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        self.map.get(key)
    }

    /// Check if a key exists.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.map.contains_key(key)
    }

    /// Remove a key.
    pub fn remove(&mut self, key: &[u8]) -> Option<V> {
        self.map.remove(key)
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Range query [start, end).
    pub fn range(&self, start: &[u8], end: &[u8]) -> impl Iterator<Item = (&[u8], &V)> {
        self.map
            .range((Bound::Included(start.to_vec()), Bound::Excluded(end.to_vec())))
            .map(|(k, v)| (k.as_slice(), v))
    }

    /// Prefix scan.
    pub fn prefix<'a>(&'a self, prefix: &[u8]) -> impl Iterator<Item = (&'a [u8], &'a V)> + 'a {
        let start = prefix.to_vec();
        let end = Self::prefix_end(prefix);
        
        let range = match end {
            Some(e) => self.map.range((Bound::Included(start), Bound::Excluded(e))),
            None => self.map.range((Bound::Included(start), Bound::Unbounded)),
        };
        
        range.map(|(k, v)| (k.as_slice(), v))
    }

    fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
        let mut end = prefix.to_vec();
        while let Some(last) = end.pop() {
            if last < 255 {
                end.push(last + 1);
                return Some(end);
            }
        }
        None
    }

    /// Memory usage (approximate).
    pub fn memory_usage(&self) -> usize {
        let key_bytes: usize = self.map.keys().map(|k| k.len() + 24).sum(); // Vec overhead
        let value_bytes = self.map.len() * std::mem::size_of::<V>();
        let btree_overhead = self.map.len() * 32; // Approximate node overhead
        key_bytes + value_bytes + btree_overhead
    }
}

impl<V> Default for SimpleKV<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_kv() {
        let mut kv: SimpleKV<u64> = SimpleKV::new();
        
        kv.insert(b"hello", 1);
        kv.insert(b"world", 2);
        
        assert_eq!(kv.get(b"hello"), Some(&1));
        assert_eq!(kv.get(b"world"), Some(&2));
        assert_eq!(kv.get(b"foo"), None);
        
        assert_eq!(kv.remove(b"hello"), Some(1));
        assert_eq!(kv.get(b"hello"), None);
    }

    #[test]
    fn test_prefix_scan() {
        let mut kv: SimpleKV<u64> = SimpleKV::new();
        
        kv.insert(b"user:1001", 1);
        kv.insert(b"user:1002", 2);
        kv.insert(b"user:1003", 3);
        kv.insert(b"post:1001", 100);
        
        let users: Vec<_> = kv.prefix(b"user:").collect();
        assert_eq!(users.len(), 3);
    }
}
