//! # MemKV - Memory-Efficient Key-Value Store
//!
//! A Rust library for storing string keys with extreme memory efficiency.
//!
//! ## Results (500K URLs, shuffled random inserts)
//!
//! | Structure | Total Memory | vs BTreeMap | Lookup/s |
//! |-----------|-------------|-------------|----------|
//! | BTreeMap | 52.0 MB | baseline | 2.1M |
//! | **InlineHot** | **34.6 MB** | **-33%** | 2.1M |
//! | FastArt | 49.1 MB | -6% | **5.2M** |
//!
//! ## Recommended Usage
//!
//! ```rust
//! // For minimum memory (-33% vs BTreeMap):
//! use memkv::InlineHot;
//! let mut map = InlineHot::new();
//! map.insert(b"user:12345", 1u64);
//! assert_eq!(map.get(b"user:12345"), Some(1));
//!
//! // For maximum speed (2x faster lookups):
//! use memkv::FastArt;
//! let mut map = FastArt::new();
//! map.insert(b"user:12345", 1u64);
//! assert_eq!(map.get(b"user:12345"), Some(1));
//! ```
//!
//! ## Structures
//!
//! - [`InlineHot`]: Best memory efficiency (22.7 B/K overhead, 12 B/K index-only)
//! - [`FastArt`]: Best lookup speed (5.2M ops/s)
//! - [`FrozenLayer`]: Immutable FST-based storage for static data
//! - [`MemKV`]: Thread-safe wrapper with generic values

#![deny(unsafe_op_in_unsafe_fn)]
#![allow(missing_docs)] // TODO: Add docs before 1.0

// Core modules
pub mod art;
pub mod art_fast;
pub mod encoding;
pub mod frozen;
pub mod simple;

// HOT implementations
pub mod hot_final;
pub mod hot_inline;

// Re-exports
pub use art::AdaptiveRadixTree;
pub use art_fast::FastArt;
pub use frozen::{FrozenLayer, FrozenLayerBuilder, FrozenStats};
pub use hot_final::HOT;
pub use hot_inline::InlineHot;
pub use simple::SimpleKV;

use parking_lot::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Memory usage statistics.
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// Bytes used by key storage
    pub key_bytes: usize,
    /// Bytes used by node structures
    pub node_bytes: usize,
    /// Bytes used by value storage
    pub value_bytes: usize,
    /// Number of keys stored
    pub num_keys: usize,
    /// Bytes per key (calculated)
    pub bytes_per_key: f64,
}

/// Configuration for MemKV.
#[derive(Debug, Clone)]
pub struct Config {
    /// Initial capacity hint
    pub initial_capacity: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            initial_capacity: 1024,
        }
    }
}

/// Thread-safe key-value store with generic values.
///
/// Uses ART internally. For u64 values, prefer [`InlineHot`] or [`FastArt`] directly.
pub struct MemKV<V> {
    art: RwLock<AdaptiveRadixTree<V>>,
    len: AtomicUsize,
}

impl<V: Clone> MemKV<V> {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            art: RwLock::new(AdaptiveRadixTree::new()),
            len: AtomicUsize::new(0),
        }
    }

    /// Insert a key-value pair. Returns previous value if key existed.
    pub fn insert(&self, key: impl AsRef<[u8]>, value: V) -> Option<V> {
        let mut art = self.art.write();
        let old = art.insert(key.as_ref(), value);
        if old.is_none() {
            self.len.fetch_add(1, Ordering::Relaxed);
        }
        old
    }

    /// Get value for key.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<V> {
        let art = self.art.read();
        art.get(key.as_ref()).cloned()
    }

    /// Check if key exists.
    pub fn contains(&self, key: impl AsRef<[u8]>) -> bool {
        self.art.read().get(key.as_ref()).is_some()
    }

    /// Remove key. Returns value if it existed.
    pub fn remove(&self, key: impl AsRef<[u8]>) -> Option<V> {
        let mut art = self.art.write();
        let old = art.remove(key.as_ref());
        if old.is_some() {
            self.len.fetch_sub(1, Ordering::Relaxed);
        }
        old
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get all keys with given prefix.
    pub fn prefix(&self, prefix: impl AsRef<[u8]>) -> Vec<(Vec<u8>, V)> {
        self.art.read().prefix_scan(prefix.as_ref()).collect()
    }
}

impl<V: Clone> Default for MemKV<V> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<V: Send> Send for MemKV<V> {}
unsafe impl<V: Send + Sync> Sync for MemKV<V> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memkv_basic() {
        let kv: MemKV<u64> = MemKV::new();
        assert!(kv.insert(b"key1", 1).is_none());
        assert_eq!(kv.get(b"key1"), Some(1));
        assert_eq!(kv.insert(b"key1", 2), Some(1));
        assert_eq!(kv.get(b"key1"), Some(2));
        assert_eq!(kv.remove(b"key1"), Some(2));
        assert_eq!(kv.get(b"key1"), None);
    }

    #[test]
    fn test_inline_hot() {
        let mut hot = InlineHot::new();
        hot.insert(b"test", 42);
        assert_eq!(hot.get(b"test"), Some(42));
    }

    #[test]
    fn test_fast_art() {
        let mut art = FastArt::new();
        art.insert(b"test", 42);
        assert_eq!(art.get(b"test"), Some(42));
    }
}
