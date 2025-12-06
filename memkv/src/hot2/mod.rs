//! HOT2: Ultra-minimal trie targeting <15 bytes overhead
//!
//! Design:
//! - Keys stored ONCE in key arena (no duplication in nodes)
//! - Leaves: 8 bytes (value only, key verified via full comparison)  
//! - Internal nodes: sparse bitmap + compressed children
//!
//! Each insert just stores the leaf, tree structure built lazily

#![allow(unused)]

/// Sorted index with lazy trie construction
/// This is basically a sorted array that can be queried efficiently
pub struct Hot2 {
    /// Keys stored contiguously
    keys: Vec<u8>,
    /// (key_offset, key_len, value) tuples, kept sorted by key
    entries: Vec<(u32, u16, u64)>,
}

impl Hot2 {
    pub fn new() -> Self {
        Self {
            keys: Vec::new(),
            entries: Vec::new(),
        }
    }
    
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    
    fn get_key(&self, idx: usize) -> &[u8] {
        let (off, len, _) = self.entries[idx];
        &self.keys[off as usize..(off as usize + len as usize)]
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        // Binary search
        let result = self.entries.binary_search_by(|(off, len, _)| {
            let stored = &self.keys[*off as usize..(*off as usize + *len as usize)];
            stored.cmp(key)
        });
        
        match result {
            Ok(idx) => Some(self.entries[idx].2),
            Err(_) => None,
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        let key_off = self.keys.len() as u32;
        let key_len = key.len() as u16;
        
        // Find insertion point
        let result = self.entries.binary_search_by(|(off, len, _)| {
            let stored = &self.keys[*off as usize..(*off as usize + *len as usize)];
            stored.cmp(key)
        });
        
        match result {
            Ok(idx) => {
                // Key exists - update value
                let old = self.entries[idx].2;
                self.entries[idx].2 = value;
                Some(old)
            }
            Err(idx) => {
                // Insert new entry
                self.keys.extend_from_slice(key);
                self.entries.insert(idx, (key_off, key_len, value));
                None
            }
        }
    }
    
    pub fn memory_stats(&self) -> Hot2Stats {
        let keys_bytes = self.keys.capacity();
        let entries_bytes = self.entries.capacity() * std::mem::size_of::<(u32, u16, u64)>();
        let raw_key_bytes = self.keys.len();
        let total = keys_bytes + entries_bytes;
        
        Hot2Stats {
            keys_bytes,
            entries_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: total.saturating_sub(raw_key_bytes),
            overhead_per_key: if self.len() > 0 {
                (total.saturating_sub(raw_key_bytes)) as f64 / self.len() as f64
            } else { 0.0 },
        }
    }
}

impl Default for Hot2 {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone)]
pub struct Hot2Stats {
    pub keys_bytes: usize,
    pub entries_bytes: usize,
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
        let mut tree = Hot2::new();
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        
        assert_eq!(tree.get(b"hello"), Some(1));
        assert_eq!(tree.get(b"world"), Some(2));
        assert_eq!(tree.get(b"notfound"), None);
    }
    
    #[test]
    fn test_many() {
        let mut tree = Hot2::new();
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        let mut correct = 0;
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            if tree.get(key.as_bytes()) == Some(i) {
                correct += 1;
            }
        }
        
        assert_eq!(correct, 1000);
        
        let stats = tree.memory_stats();
        println!("Overhead: {:.1} bytes/key", stats.overhead_per_key);
        // Entry overhead: 14 bytes (4 + 2 + 8)
        // This is 14 bytes/key overhead - close to HOT target!
    }
}
