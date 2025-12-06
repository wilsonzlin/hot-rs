//! # MemKV - Memory-Efficient Key-Value Store
//!
//! A Rust library designed for storing billions of string keys with extreme memory efficiency.
//!
//! ## Key Results (9.5M URL Dataset, 467 MB raw data)
//!
//! | Implementation | Memory | Overhead/Key | Insert ops/s |
//! |---------------|--------|--------------|--------------|
//! | **FrozenLayer (FST)** | 320 MB | **-16 bytes** | 661K |
//! | **FastArt** | **998 MB** | **58 bytes** | **5.1M** |
//! | libart (C) | 1,123 MB | 72 bytes | 4.9M |
//! | BTreeMap | 1,145 MB | 75 bytes | 3.3M |
//!
//! ## Features
//!
//! - **FrozenLayer (FST)**: Compression for immutable data (negative overhead!)
//! - **FastArt**: Best mutable ART, beats libart (C) by 19%
//! - **Point lookups**: O(key_length) lookups
//! - **Range queries**: Efficient lexicographic range iteration
//! - **Prefix scans**: Find all keys with a given prefix
//!
//! ## Example: FastArt (Mutable - Best Performance)
//!
//! ```rust
//! use memkv::FastArt;
//!
//! let mut art = FastArt::new();
//! art.insert(b"key1", 1);
//! art.insert(b"key2", 2);
//!
//! assert_eq!(art.get(b"key1"), Some(1));
//! assert_eq!(art.get(b"key2"), Some(2));
//! ```
//!
//! ## Example: Frozen Data (Best Memory Efficiency)
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
//! ## Example: Mutable Data with MemKV wrapper
//!
//! ```rust
//! use memkv::MemKV;
//!
//! let kv = MemKV::new();
//! kv.insert(b"user:1001", 42u64);
//! kv.insert(b"user:1002", 43u64);
//!
//! assert_eq!(kv.get(b"user:1001"), Some(42));
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod arena;
pub mod art;
pub mod art2;
pub mod art_arena;
pub mod art_compact;
pub mod art_compact2;
pub mod art_compact_ptr;
pub mod art_lean;
pub mod art_optimized;
pub mod art_ultra;
pub mod art_fast;
pub mod art_hot;
pub mod art_glory;
// pub mod art_minimal; // Has correctness bugs - disabled
pub mod hybrid;
pub mod hot;
pub mod hot2;
pub mod glory;
pub mod hot_proper;
// pub mod patricia; // Has infinite loop bug - disabled
pub mod encoding;
pub mod front_coded;
pub mod frozen;
pub mod simple;

#[cfg(feature = "libart")]
pub mod libart_ffi;

pub use simple::SimpleKV;
pub use art::AdaptiveRadixTree;
pub use art2::OptimizedART;
pub use art_compact::{CompactArt, KeyRef};
pub use art_compact2::{UltraCompactArt, DataRef, UltraNode};
pub use art_arena::{ArenaArt, ArenaNode, ArenaArtStats};
pub use frozen::{FrozenLayer, FrozenLayerBuilder, FrozenStats};
pub use art_optimized::{OptimizedArt, OptArtStats};
pub use art_lean::{LeanArt, LeanStats};
pub use art_ultra::{UltraArt, UltraArtStats};
pub use art_fast::FastArt;
pub use art_hot::HotArt;
pub use art_glory::{GloryArt, GloryStats};
// pub use art_minimal::{MinimalArt, MinimalStats}; // Disabled
pub use hybrid::{HybridIndex, HybridBuilder, HybridStats};
pub use hot::{TrueHot, TrueHotStats};
pub use hot2::{Hot2, Hot2Stats};
pub use glory::{Glory, GloryStats as UltimateGloryStats};
pub use hot_proper::{ProperHot, ProperHotStats};
pub use front_coded::{FrontCodedIndex, FrontCodedBuilder, FrontCodedStats};

use std::sync::atomic::{AtomicUsize, Ordering};

use parking_lot::RwLock;

/// Memory usage statistics for the store.
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    /// Total bytes used by key storage
    pub key_bytes: usize,
    /// Total bytes used by node structures
    pub node_bytes: usize,
    /// Total bytes used by value storage
    pub value_bytes: usize,
    /// Number of keys stored
    pub num_keys: usize,
    /// Bytes per key (calculated)
    pub bytes_per_key: f64,
}

/// Configuration for the MemKV store.
#[derive(Debug, Clone)]
pub struct Config {
    /// Initial capacity hint for number of keys
    pub initial_capacity: usize,
    /// Threshold for triggering compaction (delta layer size)
    pub compaction_threshold: usize,
    /// Enable automatic background compaction
    pub auto_compact: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            initial_capacity: 1024,
            compaction_threshold: 1_000_000,
            auto_compact: false,
        }
    }
}

/// A memory-efficient key-value store optimized for billions of string keys.
///
/// This is the main entry point for the library. It provides a concurrent-safe
/// interface for storing and querying key-value pairs.
///
/// Uses an Adaptive Radix Tree (ART) for memory-efficient storage with good
/// performance characteristics.
pub struct MemKV<V> {
    /// The ART backend
    art: RwLock<AdaptiveRadixTree<V>>,
    /// Number of entries
    len: AtomicUsize,
    /// Configuration
    #[allow(dead_code)]
    config: Config,
}

impl<V> MemKV<V>
where
    V: Clone,
{
    /// Create a new empty store with default configuration.
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    /// Create a new store with the given configuration.
    pub fn with_config(config: Config) -> Self {
        Self {
            art: RwLock::new(AdaptiveRadixTree::new()),
            len: AtomicUsize::new(0),
            config,
        }
    }

    /// Insert a key-value pair into the store.
    ///
    /// Returns the previous value if the key already existed.
    pub fn insert(&self, key: impl AsRef<[u8]>, value: V) -> Option<V> {
        let key = key.as_ref();
        let mut art = self.art.write();
        let old = art.insert(key, value);
        if old.is_none() {
            self.len.fetch_add(1, Ordering::Relaxed);
        }
        old
    }

    /// Get a reference to the value for a key.
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<V> {
        let key = key.as_ref();
        let art = self.art.read();
        art.get(key).cloned()
    }

    /// Check if a key exists in the store.
    pub fn contains(&self, key: impl AsRef<[u8]>) -> bool {
        let key = key.as_ref();
        let art = self.art.read();
        art.get(key).is_some()
    }

    /// Remove a key from the store.
    ///
    /// Returns the value if the key existed.
    pub fn remove(&self, key: impl AsRef<[u8]>) -> Option<V> {
        let key = key.as_ref();
        let mut art = self.art.write();
        let old = art.remove(key);
        if old.is_some() {
            self.len.fetch_sub(1, Ordering::Relaxed);
        }
        old
    }

    /// Iterate over a range of keys [start, end).
    pub fn range(&self, start: impl AsRef<[u8]>, end: impl AsRef<[u8]>) -> Vec<(Vec<u8>, V)> {
        let start = start.as_ref();
        let end = end.as_ref();
        let art = self.art.read();
        art.range((
            std::ops::Bound::Included(start),
            std::ops::Bound::Excluded(end),
        )).collect()
    }

    /// Iterate over all keys with a given prefix.
    pub fn prefix(&self, prefix: impl AsRef<[u8]>) -> Vec<(Vec<u8>, V)> {
        let prefix = prefix.as_ref();
        let art = self.art.read();
        art.prefix_scan(prefix).collect()
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
    pub fn memory_usage(&self) -> MemoryStats {
        let art = self.art.read();
        let stats = art.memory_stats();
        let num_keys = self.len();
        MemoryStats {
            key_bytes: stats.key_bytes,
            node_bytes: stats.node_bytes,
            value_bytes: stats.value_bytes,
            num_keys,
            bytes_per_key: if num_keys > 0 {
                (stats.key_bytes + stats.node_bytes + stats.value_bytes) as f64 / num_keys as f64
            } else {
                0.0
            },
        }
    }

    /// Force compaction of the delta layer into the frozen layer.
    pub fn compact(&self) {
        // TODO: Implement FST compaction
        // For now, this is a no-op
    }
}

impl<V: Clone> Default for MemKV<V> {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-safe
unsafe impl<V: Send> Send for MemKV<V> {}
unsafe impl<V: Send + Sync> Sync for MemKV<V> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let kv: MemKV<u64> = MemKV::new();

        // Insert
        assert!(kv.insert(b"key1", 1).is_none());
        assert!(kv.insert(b"key2", 2).is_none());
        assert_eq!(kv.insert(b"key1", 10), Some(1));

        // Get
        assert_eq!(kv.get(b"key1"), Some(10));
        assert_eq!(kv.get(b"key2"), Some(2));
        assert_eq!(kv.get(b"key3"), None);

        // Contains
        assert!(kv.contains(b"key1"));
        assert!(!kv.contains(b"key3"));

        // Len
        assert_eq!(kv.len(), 2);

        // Remove
        assert_eq!(kv.remove(b"key1"), Some(10));
        assert_eq!(kv.len(), 1);
        assert!(!kv.contains(b"key1"));
    }

    #[test]
    fn test_prefix_scan() {
        let kv: MemKV<u64> = MemKV::new();

        kv.insert(b"user:1001", 1);
        kv.insert(b"user:1002", 2);
        kv.insert(b"user:1003", 3);
        kv.insert(b"post:1001", 100);

        let users = kv.prefix(b"user:");
        assert_eq!(users.len(), 3);
    }

    #[test]
    fn test_empty_key() {
        let kv: MemKV<u64> = MemKV::new();
        kv.insert(b"", 42);
        assert_eq!(kv.get(b""), Some(42));
    }
}

#[cfg(test)]
mod stress_tests {
    use super::*;

    #[test]
    fn test_large_scale() {
        let kv: MemKV<u64> = MemKV::new();

        // Generate 10000 keys with varied prefixes
        let mut keys = Vec::new();
        for i in 0..10000 {
            let key = format!("domain{}.com/path/{}/item{}", i % 100, i / 100, i);
            keys.push(key);
        }

        for (i, key) in keys.iter().enumerate() {
            kv.insert(key.as_bytes(), i as u64);
        }

        assert_eq!(kv.len(), 10000);

        // Verify all
        let mut correct = 0;
        for (i, key) in keys.iter().enumerate() {
            if kv.get(key.as_bytes()) == Some(i as u64) {
                correct += 1;
            }
        }
        assert_eq!(correct, 10000, "Only {}/10000 correct", correct);
    }
}
