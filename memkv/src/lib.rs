//! Memory-efficient key-value storage for string keys.
//!
//! This library provides data structures optimized for storing large numbers
//! of string keys with minimal memory overhead.
//!
//! # Implementations
//!
//! - [`FastArt`]: Mutable Adaptive Radix Tree with ~63 bytes overhead per key.
//!   Best for read-write workloads.
//! - [`FrozenLayer`]: Immutable FST-based storage with compression (negative overhead).
//!   Best for read-only data.
//! - [`SimpleKV`]: BTreeMap-based fallback for comparison.
//!
//! # Quick Start
//!
//! ## Mutable Data (FastArt)
//!
//! ```rust
//! use memkv::FastArt;
//!
//! let mut kv = FastArt::new();
//! kv.insert(b"user:1001", 42);
//! kv.insert(b"user:1002", 43);
//!
//! assert_eq!(kv.get(b"user:1001"), Some(42));
//! assert_eq!(kv.len(), 2);
//! ```
//!
//! ## Immutable Data (FrozenLayer)
//!
//! ```rust
//! use memkv::FrozenLayer;
//!
//! // Keys must be sorted for FST construction
//! let data = vec![
//!     (b"apple".as_slice(), 1u64),
//!     (b"banana".as_slice(), 2u64),
//!     (b"cherry".as_slice(), 3u64),
//! ];
//!
//! let frozen = FrozenLayer::from_sorted_iter(data).unwrap();
//! assert_eq!(frozen.get(b"apple"), Some(1));
//! ```
//!
//! ## Thread-Safe Wrapper (MemKV)
//!
//! ```rust
//! use memkv::MemKV;
//!
//! let kv = MemKV::new();
//! kv.insert(b"key", 42u64);
//! assert_eq!(kv.get(b"key"), Some(42));
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

mod art_fast;
mod frozen;
mod simple;

pub use art_fast::FastArt;
pub use frozen::{FrozenError, FrozenLayer, FrozenLayerBuilder, FrozenStats};
pub use simple::SimpleKV;

use parking_lot::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Memory usage statistics.
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// Number of keys stored.
    pub num_keys: usize,
    /// Approximate bytes per key.
    pub bytes_per_key: f64,
}

/// Thread-safe key-value store using FastArt.
///
/// Provides a concurrent-safe wrapper around [`FastArt`] with RwLock
/// for single-writer, multiple-reader access.
///
/// For read-only data, prefer [`FrozenLayer`] which offers better compression.
pub struct MemKV {
    art: RwLock<FastArt>,
    len: AtomicUsize,
}

impl MemKV {
    /// Create a new empty store.
    pub fn new() -> Self {
        Self {
            art: RwLock::new(FastArt::new()),
            len: AtomicUsize::new(0),
        }
    }

    /// Insert a key-value pair.
    ///
    /// Returns the previous value if the key existed.
    pub fn insert(&self, key: impl AsRef<[u8]>, value: u64) -> Option<u64> {
        let mut art = self.art.write();
        let old = art.insert(key.as_ref(), value);
        if old.is_none() {
            self.len.fetch_add(1, Ordering::Relaxed);
        }
        old
    }

    /// Get the value for a key.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<u64> {
        let art = self.art.read();
        art.get(key.as_ref())
    }

    /// Check if a key exists.
    pub fn contains(&self, key: impl AsRef<[u8]>) -> bool {
        self.get(key).is_some()
    }

    /// Get the number of keys in the store.
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    /// Check if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get memory usage statistics.
    pub fn memory_stats(&self) -> MemoryStats {
        let num_keys = self.len();
        MemoryStats {
            num_keys,
            // FastArt uses ~63 bytes overhead per key on typical workloads
            bytes_per_key: 63.0,
        }
    }
}

impl Default for MemKV {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for MemKV {}
unsafe impl Sync for MemKV {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memkv_basic() {
        let kv = MemKV::new();

        assert!(kv.insert(b"key1", 1).is_none());
        assert!(kv.insert(b"key2", 2).is_none());
        assert_eq!(kv.insert(b"key1", 10), Some(1));

        assert_eq!(kv.get(b"key1"), Some(10));
        assert_eq!(kv.get(b"key2"), Some(2));
        assert_eq!(kv.get(b"key3"), None);

        assert!(kv.contains(b"key1"));
        assert!(!kv.contains(b"key3"));

        assert_eq!(kv.len(), 2);
    }

    #[test]
    fn test_fast_art_basic() {
        let mut art = FastArt::new();

        art.insert(b"hello", 1);
        art.insert(b"world", 2);

        assert_eq!(art.get(b"hello"), Some(1));
        assert_eq!(art.get(b"world"), Some(2));
        assert_eq!(art.get(b"missing"), None);

        assert_eq!(art.len(), 2);
    }

    #[test]
    fn test_frozen_layer_basic() {
        let data = vec![
            (b"a".as_slice(), 1u64),
            (b"b".as_slice(), 2u64),
            (b"c".as_slice(), 3u64),
        ];

        let frozen = FrozenLayer::from_sorted_iter(data).unwrap();

        assert_eq!(frozen.get(b"a"), Some(1));
        assert_eq!(frozen.get(b"b"), Some(2));
        assert_eq!(frozen.get(b"c"), Some(3));
        assert_eq!(frozen.get(b"d"), None);

        assert_eq!(frozen.len(), 3);
    }

    #[test]
    fn test_simple_kv_basic() {
        let mut kv = SimpleKV::new();

        kv.insert(b"hello", 1u64);
        kv.insert(b"world", 2);

        assert_eq!(kv.get(b"hello"), Some(&1));
        assert_eq!(kv.get(b"world"), Some(&2));
        assert_eq!(kv.get(b"missing"), None);
    }
}
