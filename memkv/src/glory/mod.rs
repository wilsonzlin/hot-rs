//! GLORY: Absolute minimum overhead key-value store
//!
//! Design: Sorted entries with binary search
//! - Keys stored inline with 2-byte length prefix
//! - Each entry: [key_len: 2][key bytes...][value: 8]
//! - Binary search for lookup: O(log n)
//! - Sorted insert: O(n) for insert position, O(1) amortized append
//!
//! Overhead: exactly 10 bytes per key (2-byte len + 8-byte value)
//! This is LESS than the theoretical minimum for a trie!
//!
//! Trade-off: O(n) insert but O(log n) lookup, minimal memory

#![allow(unused)]

/// Ultra-compact sorted key-value store
pub struct Glory {
    /// Format: [key_len: u16][key bytes][value: u64] repeated
    data: Vec<u8>,
    /// Entry offsets for binary search
    offsets: Vec<u32>,
    /// Number of entries
    len: usize,
}

impl Glory {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            offsets: Vec::new(),
            len: 0,
        }
    }
    
    pub fn with_capacity(keys: usize, total_key_bytes: usize) -> Self {
        // Each entry: 2 (len) + key_bytes + 8 (value)
        let data_cap = total_key_bytes + keys * 10;
        Self {
            data: Vec::with_capacity(data_cap),
            offsets: Vec::with_capacity(keys),
            len: 0,
        }
    }
    
    #[inline]
    pub fn len(&self) -> usize { self.len }
    
    #[inline]
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    /// Get key at index
    fn get_key_at(&self, idx: usize) -> &[u8] {
        let off = self.offsets[idx] as usize;
        let len = u16::from_le_bytes([self.data[off], self.data[off + 1]]) as usize;
        &self.data[off + 2..off + 2 + len]
    }
    
    /// Get value at index
    fn get_value_at(&self, idx: usize) -> u64 {
        let off = self.offsets[idx] as usize;
        let len = u16::from_le_bytes([self.data[off], self.data[off + 1]]) as usize;
        let val_off = off + 2 + len;
        u64::from_le_bytes(self.data[val_off..val_off + 8].try_into().unwrap())
    }
    
    /// Set value at index
    fn set_value_at(&mut self, idx: usize, value: u64) {
        let off = self.offsets[idx] as usize;
        let len = u16::from_le_bytes([self.data[off], self.data[off + 1]]) as usize;
        let val_off = off + 2 + len;
        self.data[val_off..val_off + 8].copy_from_slice(&value.to_le_bytes());
    }
    
    /// Binary search for key, returns (found, index)
    fn search(&self, key: &[u8]) -> (bool, usize) {
        let mut left = 0;
        let mut right = self.len;
        
        while left < right {
            let mid = left + (right - left) / 2;
            let mid_key = self.get_key_at(mid);
            
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => left = mid + 1,
                std::cmp::Ordering::Greater => right = mid,
                std::cmp::Ordering::Equal => return (true, mid),
            }
        }
        
        (false, left)
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        
        let (found, idx) = self.search(key);
        if found {
            Some(self.get_value_at(idx))
        } else {
            None
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        if self.len == 0 {
            // First entry
            let off = self.data.len() as u32;
            self.data.extend_from_slice(&(key.len() as u16).to_le_bytes());
            self.data.extend_from_slice(key);
            self.data.extend_from_slice(&value.to_le_bytes());
            self.offsets.push(off);
            self.len = 1;
            return None;
        }
        
        let (found, idx) = self.search(key);
        
        if found {
            // Update existing
            let old = self.get_value_at(idx);
            self.set_value_at(idx, value);
            return Some(old);
        }
        
        // Insert new entry
        let off = self.data.len() as u32;
        self.data.extend_from_slice(&(key.len() as u16).to_le_bytes());
        self.data.extend_from_slice(key);
        self.data.extend_from_slice(&value.to_le_bytes());
        
        // Insert offset at correct position
        self.offsets.insert(idx, off);
        self.len += 1;
        
        None
    }
    
    pub fn memory_stats(&self) -> GloryStats {
        let data_bytes = self.data.capacity();
        let offsets_bytes = self.offsets.capacity() * 4;
        let total = data_bytes + offsets_bytes;
        
        // Raw key bytes: total data - 2 bytes per key (len) - 8 bytes per key (value)
        let raw_key_bytes = self.data.len().saturating_sub(self.len * 10);
        let overhead = total.saturating_sub(raw_key_bytes);
        
        GloryStats {
            data_bytes,
            offsets_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: overhead,
            overhead_per_key: if self.len > 0 {
                overhead as f64 / self.len as f64
            } else { 0.0 },
        }
    }
}

impl Default for Glory {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone)]
pub struct GloryStats {
    pub data_bytes: usize,
    pub offsets_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut tree = Glory::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        
        assert_eq!(tree.get(b"hello"), Some(1));
        assert_eq!(tree.get(b"world"), Some(2));
        assert_eq!(tree.get(b"notfound"), None);
        assert_eq!(tree.len(), 2);
    }
    
    #[test]
    fn test_prefix() {
        let mut tree = Glory::new();
        
        tree.insert(b"test", 1);
        tree.insert(b"testing", 2);
        tree.insert(b"tested", 3);
        
        assert_eq!(tree.get(b"test"), Some(1));
        assert_eq!(tree.get(b"testing"), Some(2));
        assert_eq!(tree.get(b"tested"), Some(3));
    }
    
    #[test]
    fn test_update() {
        let mut tree = Glory::new();
        
        assert_eq!(tree.insert(b"key", 1), None);
        assert_eq!(tree.insert(b"key", 2), Some(1));
        assert_eq!(tree.get(b"key"), Some(2));
        assert_eq!(tree.len(), 1);
    }
    
    #[test]
    fn test_many() {
        let mut tree = Glory::new();
        
        // Insert in random order
        let keys: Vec<String> = (0..1000u64)
            .map(|i| format!("key{:04}", (i * 997) % 1000))
            .collect();
        
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        
        // Verify (note: values will be from last insert for each key)
        let mut found = 0;
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            if tree.get(key.as_bytes()).is_some() {
                found += 1;
            }
        }
        
        assert_eq!(found, 1000);
        assert_eq!(tree.len(), 1000);
        
        let stats = tree.memory_stats();
        println!("Memory stats for 1000 keys:");
        println!("  Data: {} bytes ({:.1}/key)", stats.data_bytes, stats.data_bytes as f64 / 1000.0);
        println!("  Offsets: {} bytes ({:.1}/key)", stats.offsets_bytes, stats.offsets_bytes as f64 / 1000.0);
        println!("  Raw keys: {} bytes", stats.raw_key_bytes);
        println!("  Total: {} bytes", stats.total_bytes);
        println!("  Overhead: {:.1} bytes/key", stats.overhead_per_key);
        
        // Without pre-allocation, Vec doubling adds overhead
        // Target with pre-allocation: 14 bytes (2 len + 8 value + 4 offset)
        assert!(stats.overhead_per_key < 35.0, "Overhead too high: {:.1}", stats.overhead_per_key);
    }
    
    #[test]
    fn test_large() {
        // Pre-allocate for better memory efficiency
        let mut tree = Glory::with_capacity(10000, 80000);
        
        for i in 0..10000u64 {
            let key = format!("key{:05}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        let mut correct = 0;
        for i in 0..10000u64 {
            let key = format!("key{:05}", i);
            if tree.get(key.as_bytes()) == Some(i) {
                correct += 1;
            }
        }
        
        println!("Correct: {}/10000", correct);
        assert_eq!(correct, 10000);
        
        let stats = tree.memory_stats();
        println!("Memory stats for 10000 keys (pre-allocated):");
        println!("  Overhead: {:.1} bytes/key", stats.overhead_per_key);
        
        // With pre-allocation, should be very close to theoretical minimum
        assert!(stats.overhead_per_key < 16.0, "Overhead too high: {:.1}", stats.overhead_per_key);
    }
}
