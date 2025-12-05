//! Front-Coded (Prefix-Compressed) Index
//!
//! Front coding stores sorted keys by sharing common prefixes:
//! - First key in each block: stored in full
//! - Subsequent keys: store (shared_prefix_len, suffix)
//!
//! This achieves 25-80% compression depending on prefix sharing.
//! Ideal for hierarchical keys like paths and URLs.
//!
//! Key properties:
//! - Must be built from sorted keys (bulk load)
//! - Immutable after construction
//! - O(log n) lookups using binary search on blocks
//! - O(block_size) scan within each block

use std::cmp::Ordering;

/// Block size for front coding (number of keys per block)
/// Larger blocks = better compression, slower lookups
const BLOCK_SIZE: usize = 16;

/// Maximum prefix length to store inline
const MAX_PREFIX: usize = 256;

/// A front-coded string block
/// 
/// Layout:
/// - First key is stored in full
/// - Remaining keys: (prefix_len: u8, suffix...)
struct FrontCodedBlock {
    /// Offset into data arena where this block starts
    data_offset: u32,
    /// Total bytes in this block's data
    data_len: u32,
    /// Number of keys in this block (1 to BLOCK_SIZE)
    num_keys: u8,
    /// Offset into values array
    values_offset: u32,
}

/// Front-coded index for sorted string keys
/// 
/// Provides excellent compression for keys with high prefix sharing.
/// Trade-off: immutable, requires sorted input.
pub struct FrontCodedIndex<V> {
    /// All key data (front-coded)
    data: Vec<u8>,
    /// Block headers
    blocks: Vec<FrontCodedBlock>,
    /// Values (one per key)
    values: Vec<V>,
    /// First key of each block (for binary search)
    block_first_keys: Vec<Vec<u8>>,
    /// Total number of keys
    len: usize,
}

/// Builder for FrontCodedIndex
pub struct FrontCodedBuilder<V> {
    data: Vec<u8>,
    blocks: Vec<FrontCodedBlock>,
    values: Vec<V>,
    block_first_keys: Vec<Vec<u8>>,
    current_block_keys: Vec<Vec<u8>>,
    current_block_values: Vec<V>,
    last_key: Vec<u8>,
    len: usize,
}

impl<V: Clone> FrontCodedBuilder<V> {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            blocks: Vec::new(),
            values: Vec::new(),
            block_first_keys: Vec::new(),
            current_block_keys: Vec::new(),
            current_block_values: Vec::new(),
            last_key: Vec::new(),
            len: 0,
        }
    }
    
    /// Add a key-value pair
    /// 
    /// **Keys must be added in sorted order!**
    pub fn add(&mut self, key: &[u8], value: V) {
        // Validate sorted order
        debug_assert!(
            self.len == 0 || key >= self.last_key.as_slice(),
            "Keys must be sorted"
        );
        
        self.current_block_keys.push(key.to_vec());
        self.current_block_values.push(value);
        self.last_key = key.to_vec();
        self.len += 1;
        
        // Flush block when full
        if self.current_block_keys.len() >= BLOCK_SIZE {
            self.flush_block();
        }
    }
    
    fn flush_block(&mut self) {
        if self.current_block_keys.is_empty() {
            return;
        }
        
        let data_offset = self.data.len() as u32;
        let values_offset = self.values.len() as u32;
        let num_keys = self.current_block_keys.len() as u8;
        
        // Store first key of block for binary search
        self.block_first_keys.push(self.current_block_keys[0].clone());
        
        // Encode first key in full: length (varint) + bytes
        let first_key = self.current_block_keys[0].clone();
        self.write_varint(first_key.len());
        self.data.extend_from_slice(&first_key);
        
        // Encode remaining keys with prefix compression
        for i in 1..self.current_block_keys.len() {
            let (prefix_len, suffix_len, suffix) = {
                let prev = &self.current_block_keys[i - 1];
                let curr = &self.current_block_keys[i];
                
                // Find common prefix length
                let prefix_len = prev.iter()
                    .zip(curr.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
                    .min(MAX_PREFIX);
                
                let suffix_len = curr.len() - prefix_len;
                let suffix = curr[prefix_len..].to_vec();
                
                (prefix_len, suffix_len, suffix)
            };
            
            // Write prefix length + suffix
            self.write_varint(prefix_len);
            self.write_varint(suffix_len);
            self.data.extend_from_slice(&suffix);
        }
        
        let data_len = (self.data.len() as u32) - data_offset;
        
        // Move values to main array
        self.values.append(&mut self.current_block_values);
        
        // Record block
        self.blocks.push(FrontCodedBlock {
            data_offset,
            data_len,
            num_keys,
            values_offset,
        });
        
        self.current_block_keys.clear();
    }
    
    fn write_varint(&mut self, mut value: usize) {
        while value >= 128 {
            self.data.push((value & 0x7F) as u8 | 0x80);
            value >>= 7;
        }
        self.data.push(value as u8);
    }
    
    /// Finish building and return the index
    pub fn finish(mut self) -> FrontCodedIndex<V> {
        self.flush_block();
        
        FrontCodedIndex {
            data: self.data,
            blocks: self.blocks,
            values: self.values,
            block_first_keys: self.block_first_keys,
            len: self.len,
        }
    }
}

impl<V: Clone> Default for FrontCodedBuilder<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> FrontCodedIndex<V> {
    /// Build from sorted key-value pairs
    pub fn from_sorted_iter<'a, I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (&'a [u8], V)>,
    {
        let mut builder = FrontCodedBuilder::new();
        for (key, value) in iter {
            builder.add(key, value);
        }
        builder.finish()
    }
    
    /// Number of keys
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }
    
    /// Check if empty
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    
    /// Look up a key
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.blocks.is_empty() {
            return None;
        }
        
        // Binary search to find the block
        let block_idx = self.find_block(key)?;
        let block = &self.blocks[block_idx];
        
        // Decode and search within block
        self.search_in_block(block, key)
    }
    
    fn find_block(&self, key: &[u8]) -> Option<usize> {
        // Binary search on block first keys
        let idx = self.block_first_keys.partition_point(|first_key| {
            first_key.as_slice() <= key
        });
        
        if idx == 0 {
            // Key might be in first block
            Some(0)
        } else {
            // Key might be in block idx-1
            Some(idx - 1)
        }
    }
    
    fn search_in_block(&self, block: &FrontCodedBlock, key: &[u8]) -> Option<&V> {
        let data = &self.data[block.data_offset as usize..(block.data_offset + block.data_len) as usize];
        let mut pos = 0;
        let mut prev_key = Vec::new();
        
        for i in 0..block.num_keys as usize {
            let (current_key, new_pos) = if i == 0 {
                // First key: full key
                let (key_len, p) = self.read_varint(data, pos);
                let k = data[p..p + key_len].to_vec();
                (k, p + key_len)
            } else {
                // Subsequent keys: prefix + suffix
                let (prefix_len, p1) = self.read_varint(data, pos);
                let (suffix_len, p2) = self.read_varint(data, p1);
                
                let mut k = prev_key[..prefix_len].to_vec();
                k.extend_from_slice(&data[p2..p2 + suffix_len]);
                (k, p2 + suffix_len)
            };
            
            match current_key.as_slice().cmp(key) {
                Ordering::Equal => {
                    let value_idx = block.values_offset as usize + i;
                    return Some(&self.values[value_idx]);
                }
                Ordering::Greater => {
                    // Keys are sorted, so if we passed the key, it's not here
                    return None;
                }
                Ordering::Less => {}
            }
            
            prev_key = current_key;
            pos = new_pos;
        }
        
        None
    }
    
    fn read_varint(&self, data: &[u8], pos: usize) -> (usize, usize) {
        let mut value = 0usize;
        let mut shift = 0;
        let mut p = pos;
        
        loop {
            let byte = data[p];
            value |= ((byte & 0x7F) as usize) << shift;
            p += 1;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        
        (value, p)
    }
    
    /// Memory usage statistics
    pub fn memory_stats(&self) -> FrontCodedStats {
        let data_bytes = self.data.capacity();
        let blocks_bytes = self.blocks.capacity() * std::mem::size_of::<FrontCodedBlock>();
        let values_bytes = self.values.capacity() * std::mem::size_of::<V>();
        let first_keys_bytes: usize = self.block_first_keys.iter()
            .map(|k| k.capacity())
            .sum();
        
        FrontCodedStats {
            data_bytes,
            blocks_bytes,
            values_bytes,
            first_keys_bytes,
            total_bytes: data_bytes + blocks_bytes + values_bytes + first_keys_bytes,
            num_blocks: self.blocks.len(),
            num_keys: self.len,
            bytes_per_key: if self.len > 0 {
                (data_bytes + blocks_bytes + values_bytes + first_keys_bytes) as f64 / self.len as f64
            } else {
                0.0
            },
        }
    }
}

/// Memory statistics for front-coded index
#[derive(Debug, Clone)]
pub struct FrontCodedStats {
    /// Bytes used for key data
    pub data_bytes: usize,
    /// Bytes used for block headers
    pub blocks_bytes: usize,
    /// Bytes used for values
    pub values_bytes: usize,
    /// Bytes used for first keys (binary search)
    pub first_keys_bytes: usize,
    /// Total memory usage
    pub total_bytes: usize,
    /// Number of blocks
    pub num_blocks: usize,
    /// Number of keys
    pub num_keys: usize,
    /// Average bytes per key
    pub bytes_per_key: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let data = vec![
            (b"apple".as_slice(), 1u64),
            (b"banana".as_slice(), 2u64),
            (b"cherry".as_slice(), 3u64),
        ];
        
        let index = FrontCodedIndex::from_sorted_iter(data);
        
        assert_eq!(index.get(b"apple"), Some(&1));
        assert_eq!(index.get(b"banana"), Some(&2));
        assert_eq!(index.get(b"cherry"), Some(&3));
        assert_eq!(index.get(b"grape"), None);
        assert_eq!(index.len(), 3);
    }
    
    #[test]
    fn test_prefix_sharing() {
        let data = vec![
            (b"document/1/author".as_slice(), 1u64),
            (b"document/1/content".as_slice(), 2u64),
            (b"document/1/title".as_slice(), 3u64),
            (b"document/2/author".as_slice(), 4u64),
            (b"document/2/content".as_slice(), 5u64),
        ];
        
        let index = FrontCodedIndex::from_sorted_iter(data);
        
        assert_eq!(index.get(b"document/1/author"), Some(&1));
        assert_eq!(index.get(b"document/2/content"), Some(&5));
        
        let stats = index.memory_stats();
        println!("Data bytes: {}", stats.data_bytes);
        println!("Bytes per key: {:.1}", stats.bytes_per_key);
    }
    
    #[test]
    fn test_urls() {
        let urls: Vec<Vec<u8>> = vec![
            b"https://example.com/path/1".to_vec(),
            b"https://example.com/path/2".to_vec(),
            b"https://example.com/path/3".to_vec(),
            b"https://test.org/page/a".to_vec(),
            b"https://test.org/page/b".to_vec(),
        ];
        
        let data: Vec<_> = urls.iter()
            .enumerate()
            .map(|(i, url)| (url.as_slice(), i as u64))
            .collect();
        
        let index = FrontCodedIndex::from_sorted_iter(data);
        
        for (i, url) in urls.iter().enumerate() {
            assert_eq!(index.get(url), Some(&(i as u64)), "Failed for {}", String::from_utf8_lossy(url));
        }
        
        let stats = index.memory_stats();
        println!("URLs compression:");
        println!("  Data bytes: {}", stats.data_bytes);
        println!("  Bytes per key: {:.1}", stats.bytes_per_key);
        
        // Calculate raw size for comparison
        let raw_size: usize = urls.iter().map(|u| u.len()).sum();
        println!("  Raw size: {} bytes", raw_size);
        println!("  Compression ratio: {:.2}x", raw_size as f64 / stats.data_bytes as f64);
    }
    
    #[test]
    fn test_many_keys() {
        let keys: Vec<Vec<u8>> = (0..1000)
            .map(|i| format!("prefix/middle/suffix/{:05}", i).into_bytes())
            .collect();
        
        let data: Vec<_> = keys.iter()
            .enumerate()
            .map(|(i, k)| (k.as_slice(), i as u64))
            .collect();
        
        let index = FrontCodedIndex::from_sorted_iter(data);
        
        assert_eq!(index.len(), 1000);
        
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(index.get(key), Some(&(i as u64)));
        }
        
        let stats = index.memory_stats();
        println!("1000 keys with prefix sharing:");
        println!("  Bytes per key: {:.1}", stats.bytes_per_key);
    }
}
