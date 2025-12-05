//! Frozen layer using FST (Finite State Transducer) for extreme compression.
//!
//! The FST layer stores sorted, immutable key-value data with very low memory overhead.
//! It's ideal for stable data that doesn't change frequently.
//!
//! Key features:
//! - Extreme compression: ~3-5 bytes per key for typical data
//! - Fast lookups: O(key length) 
//! - Range queries: O(1) to get iterator
//! - Prefix scans: O(1) to get iterator
//!
//! Trade-offs:
//! - Immutable once built
//! - Values must fit in u64 (use as pointer to separate value storage)
//! - Must provide sorted input during construction

use fst::{Map, MapBuilder, IntoStreamer, Streamer};

/// Error type for frozen layer operations.
#[derive(Debug)]
pub enum FrozenError {
    /// FST construction or access error.
    Fst(fst::Error),
}

impl From<fst::Error> for FrozenError {
    fn from(e: fst::Error) -> Self {
        FrozenError::Fst(e)
    }
}

impl std::fmt::Display for FrozenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrozenError::Fst(e) => write!(f, "FST error: {}", e),
        }
    }
}

impl std::error::Error for FrozenError {}

/// Result type for frozen layer operations.
pub type Result<T> = std::result::Result<T, FrozenError>;

/// A frozen key-value store using FST for extreme compression.
/// 
/// Keys are stored in the FST structure, values are u64 (can be used as indices
/// into a separate value storage if needed).
pub struct FrozenLayer {
    /// The FST map storing key -> value mappings.
    fst: Map<Vec<u8>>,
    /// Memory used by the FST.
    fst_size: usize,
    /// Number of entries.
    len: usize,
}

/// Builder for constructing a FrozenLayer from sorted input.
pub struct FrozenLayerBuilder {
    builder: MapBuilder<Vec<u8>>,
    count: usize,
}

/// Statistics about the frozen layer.
#[derive(Debug, Clone, Default)]
pub struct FrozenStats {
    /// Total bytes used by the FST.
    pub fst_bytes: usize,
    /// Number of keys stored.
    pub key_count: usize,
    /// Average bytes per key.
    pub bytes_per_key: f64,
}

impl FrozenLayerBuilder {
    /// Create a new builder.
    pub fn new() -> Result<Self> {
        let builder = MapBuilder::memory();
        Ok(Self { builder, count: 0 })
    }
    
    /// Insert a key-value pair.
    /// 
    /// **Keys must be inserted in lexicographic order!**
    pub fn insert(&mut self, key: &[u8], value: u64) -> Result<()> {
        self.builder.insert(key, value)?;
        self.count += 1;
        Ok(())
    }
    
    /// Finish building and return the frozen layer.
    pub fn finish(self) -> Result<FrozenLayer> {
        let bytes = self.builder.into_inner()?;
        let fst_size = bytes.len();
        let fst = Map::new(bytes)?;
        Ok(FrozenLayer {
            fst,
            fst_size,
            len: self.count,
        })
    }
}

impl Default for FrozenLayerBuilder {
    fn default() -> Self {
        Self::new().expect("Failed to create builder")
    }
}

impl FrozenLayer {
    /// Build a frozen layer from sorted key-value pairs.
    /// 
    /// **Keys must be sorted lexicographically!**
    pub fn from_sorted_iter<'a, I>(iter: I) -> Result<Self>
    where
        I: IntoIterator<Item = (&'a [u8], u64)>,
    {
        let mut builder = FrozenLayerBuilder::new()?;
        for (key, value) in iter {
            builder.insert(key, value)?;
        }
        builder.finish()
    }
    
    /// Get the value for a key.
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        self.fst.get(key)
    }
    
    /// Check if a key exists.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.fst.contains_key(key)
    }
    
    /// Get number of keys.
    pub fn len(&self) -> usize {
        self.len
    }
    
    /// Returns true if the layer contains no keys.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    
    /// Get memory statistics.
    pub fn stats(&self) -> FrozenStats {
        FrozenStats {
            fst_bytes: self.fst_size,
            key_count: self.len,
            bytes_per_key: if self.len > 0 {
                self.fst_size as f64 / self.len as f64
            } else {
                0.0
            },
        }
    }
    
    /// Iterate over all key-value pairs.
    pub fn iter(&self) -> impl Iterator<Item = (Vec<u8>, u64)> + '_ {
        let mut stream = self.fst.stream();
        std::iter::from_fn(move || {
            stream.next().map(|(k, v)| (k.to_vec(), v))
        })
    }
    
    /// Iterate over keys in a range [start, end).
    pub fn range(&self, start: &[u8], end: &[u8]) -> Vec<(Vec<u8>, u64)> {
        let mut stream = self.fst.range().ge(start).lt(end).into_stream();
        let mut results = Vec::new();
        while let Some((k, v)) = stream.next() {
            results.push((k.to_vec(), v));
        }
        results
    }
    
    /// Iterate over keys with a prefix.
    pub fn prefix_scan(&self, prefix: &[u8]) -> Vec<(Vec<u8>, u64)> {
        // Calculate the upper bound for prefix scan
        let mut end = prefix.to_vec();
        // Increment the last byte, handling overflow
        let mut i = end.len();
        while i > 0 {
            i -= 1;
            if end[i] < 255 {
                end[i] += 1;
                break;
            } else {
                end.pop();
            }
        }
        
        if end.is_empty() {
            // Prefix is all 0xFF bytes, scan to end
            let mut stream = self.fst.range().ge(prefix).into_stream();
            let mut results = Vec::new();
            while let Some((k, v)) = stream.next() {
                results.push((k.to_vec(), v));
            }
            results
        } else {
            self.range(prefix, &end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_operations() {
        // Keys must be sorted
        let data = vec![
            (b"apple".as_slice(), 1u64),
            (b"banana".as_slice(), 2u64),
            (b"cherry".as_slice(), 3u64),
        ];
        
        let layer = FrozenLayer::from_sorted_iter(data).unwrap();
        
        assert_eq!(layer.get(b"apple"), Some(1));
        assert_eq!(layer.get(b"banana"), Some(2));
        assert_eq!(layer.get(b"cherry"), Some(3));
        assert_eq!(layer.get(b"grape"), None);
        
        assert_eq!(layer.len(), 3);
    }
    
    #[test]
    fn test_prefix_scan() {
        let data = vec![
            (b"post:1".as_slice(), 100u64),
            (b"user:1".as_slice(), 1u64),
            (b"user:2".as_slice(), 2u64),
            (b"user:3".as_slice(), 3u64),
        ];
        
        let layer = FrozenLayer::from_sorted_iter(data).unwrap();
        
        let users = layer.prefix_scan(b"user:");
        assert_eq!(users.len(), 3);
    }
    
    #[test]
    fn test_compression() {
        // Generate 1000 sorted keys
        let data: Vec<(Vec<u8>, u64)> = (0..1000u64)
            .map(|i| (format!("key{:05}", i).into_bytes(), i))
            .collect();
        
        let raw_key_size: usize = data.iter().map(|(k, _)| k.len()).sum();
        
        let layer = FrozenLayer::from_sorted_iter(
            data.iter().map(|(k, v)| (k.as_slice(), *v))
        ).unwrap();
        
        let stats = layer.stats();
        
        println!("Raw key data: {} bytes", raw_key_size);
        println!("FST size: {} bytes", stats.fst_bytes);
        println!("Compression ratio: {:.2}x", raw_key_size as f64 / stats.fst_bytes as f64);
        println!("Bytes per key: {:.1}", stats.bytes_per_key);
        
        // FST should compress well for sequential keys
        assert!(stats.fst_bytes < raw_key_size);
    }
    
    #[test]
    fn test_range_query() {
        let data = vec![
            (b"a".as_slice(), 1u64),
            (b"b".as_slice(), 2u64),
            (b"c".as_slice(), 3u64),
            (b"d".as_slice(), 4u64),
            (b"e".as_slice(), 5u64),
        ];
        
        let layer = FrozenLayer::from_sorted_iter(data).unwrap();
        
        let range = layer.range(b"b", b"e");
        assert_eq!(range.len(), 3); // b, c, d
        assert_eq!(range[0].1, 2);
        assert_eq!(range[2].1, 4);
    }
}
