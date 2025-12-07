//! LSM-HOT: Hybrid structure achieving near-10 bytes/key overhead
//!
//! Strategy:
//! - Mutable buffer: Small ART for recent inserts
//! - Frozen layers: Sorted arrays with minimal overhead
//! - Periodic compaction: Merge buffer into frozen layers
//!
//! Overhead target:
//! - Frozen layer: ~6 bytes/key (len + offset)
//! - Buffer: ~35 bytes/key but only for small fraction of keys
//! - Amortized: ~10 bytes/key when buffer is small relative to total

/// Frozen layer: sorted array with minimal overhead
/// Layout: [keys: contiguous][offsets: 4 bytes each][values: 8 bytes each]
struct FrozenLayer {
    key_data: Vec<u8>,        // Contiguous keys
    offsets: Vec<u32>,        // Start offset of each key  
    key_lens: Vec<u16>,       // Length of each key
    values: Vec<u64>,         // Values
}

impl FrozenLayer {
    fn new() -> Self {
        Self {
            key_data: Vec::new(),
            offsets: Vec::new(),
            key_lens: Vec::new(),
            values: Vec::new(),
        }
    }
    
    fn len(&self) -> usize {
        self.values.len()
    }
    
    fn get_key(&self, idx: usize) -> &[u8] {
        let start = self.offsets[idx] as usize;
        let len = self.key_lens[idx] as usize;
        &self.key_data[start..start + len]
    }
    
    fn get(&self, key: &[u8]) -> Option<u64> {
        // Binary search
        let mut lo = 0;
        let mut hi = self.values.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.get_key(mid).cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(self.values[mid]),
            }
        }
        None
    }
    
    /// Build from sorted (key, value) pairs
    fn from_sorted(pairs: Vec<(Vec<u8>, u64)>) -> Self {
        let mut layer = Self::new();
        layer.offsets.reserve(pairs.len());
        layer.key_lens.reserve(pairs.len());
        layer.values.reserve(pairs.len());
        
        let total_key_bytes: usize = pairs.iter().map(|(k, _)| k.len()).sum();
        layer.key_data.reserve(total_key_bytes);
        
        for (key, value) in pairs {
            layer.offsets.push(layer.key_data.len() as u32);
            layer.key_lens.push(key.len() as u16);
            layer.key_data.extend_from_slice(&key);
            layer.values.push(value);
        }
        layer
    }
    
    fn memory_usage(&self) -> usize {
        self.key_data.capacity() + 
        self.offsets.capacity() * 4 +
        self.key_lens.capacity() * 2 +
        self.values.capacity() * 8
    }
    
    fn raw_key_bytes(&self) -> usize {
        self.key_data.len()
    }
}

/// Mutable buffer using simple sorted Vec (for small sizes)
struct MutableBuffer {
    entries: Vec<(Vec<u8>, u64)>,  // (key, value) pairs
    sorted: bool,
}

impl MutableBuffer {
    fn new() -> Self {
        Self { entries: Vec::new(), sorted: true }
    }
    
    fn len(&self) -> usize {
        self.entries.len()
    }
    
    fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        // Check if key exists (linear scan for small buffer)
        for (k, v) in &mut self.entries {
            if k.as_slice() == key {
                let old = *v;
                *v = value;
                return Some(old);
            }
        }
        self.entries.push((key.to_vec(), value));
        self.sorted = false;
        None
    }
    
    fn get(&self, key: &[u8]) -> Option<u64> {
        for (k, v) in &self.entries {
            if k.as_slice() == key {
                return Some(*v);
            }
        }
        None
    }
    
    fn take_sorted(&mut self) -> Vec<(Vec<u8>, u64)> {
        if !self.sorted {
            self.entries.sort_by(|a, b| a.0.cmp(&b.0));
            self.sorted = true;
        }
        std::mem::take(&mut self.entries)
    }
    
    fn memory_usage(&self) -> usize {
        self.entries.iter().map(|(k, _)| k.capacity() + 32 + 8).sum()
    }
}

/// LSM-HOT: Low-overhead hybrid structure
pub struct LsmHot {
    buffer: MutableBuffer,
    frozen: Option<FrozenLayer>,
    buffer_limit: usize,
}

impl LsmHot {
    pub fn new() -> Self {
        Self {
            buffer: MutableBuffer::new(),
            frozen: None,
            buffer_limit: 10000, // Compact when buffer reaches this size
        }
    }
    
    pub fn with_buffer_limit(limit: usize) -> Self {
        Self {
            buffer: MutableBuffer::new(),
            frozen: None,
            buffer_limit: limit,
        }
    }
    
    pub fn len(&self) -> usize {
        self.buffer.len() + self.frozen.as_ref().map_or(0, |f| f.len())
    }
    
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        // Check frozen layer first
        if let Some(ref frozen) = self.frozen {
            if frozen.get(key).is_some() {
                // Key exists in frozen - need to update
                // For simplicity, just insert into buffer (newer wins on lookup)
            }
        }
        
        let result = self.buffer.insert(key, value);
        
        // Compact if buffer is too large
        if self.buffer.len() >= self.buffer_limit {
            self.compact();
        }
        
        result
    }
    
    fn compact(&mut self) {
        let buffer_entries = self.buffer.take_sorted();
        
        if let Some(frozen) = self.frozen.take() {
            // Merge buffer with frozen layer
            let mut merged = Vec::with_capacity(frozen.len() + buffer_entries.len());
            
            let mut fi = 0;
            let mut bi = 0;
            
            while fi < frozen.len() && bi < buffer_entries.len() {
                let fk = frozen.get_key(fi);
                let bk = &buffer_entries[bi].0;
                
                match fk.cmp(bk.as_slice()) {
                    std::cmp::Ordering::Less => {
                        merged.push((fk.to_vec(), frozen.values[fi]));
                        fi += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        merged.push(buffer_entries[bi].clone());
                        bi += 1;
                    }
                    std::cmp::Ordering::Equal => {
                        // Buffer wins (newer)
                        merged.push(buffer_entries[bi].clone());
                        fi += 1;
                        bi += 1;
                    }
                }
            }
            
            while fi < frozen.len() {
                merged.push((frozen.get_key(fi).to_vec(), frozen.values[fi]));
                fi += 1;
            }
            
            while bi < buffer_entries.len() {
                merged.push(buffer_entries[bi].clone());
                bi += 1;
            }
            
            self.frozen = Some(FrozenLayer::from_sorted(merged));
        } else {
            self.frozen = Some(FrozenLayer::from_sorted(buffer_entries));
        }
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        // Check buffer first (contains updates)
        if let Some(v) = self.buffer.get(key) {
            return Some(v);
        }
        // Then check frozen layer
        if let Some(ref frozen) = self.frozen {
            return frozen.get(key);
        }
        None
    }
    
    pub fn memory_usage(&self) -> usize {
        self.buffer.memory_usage() + 
        self.frozen.as_ref().map_or(0, |f| f.memory_usage())
    }
    
    pub fn raw_key_bytes(&self) -> usize {
        let buffer_keys: usize = self.buffer.entries.iter().map(|(k, _)| k.len()).sum();
        let frozen_keys = self.frozen.as_ref().map_or(0, |f| f.raw_key_bytes());
        buffer_keys + frozen_keys
    }
}

impl Default for LsmHot {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut t = LsmHot::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        assert_eq!(t.get(b"hello"), Some(1));
        assert_eq!(t.get(b"world"), Some(2));
        assert_eq!(t.get(b"missing"), None);
    }
    
    #[test]
    fn test_update() {
        let mut t = LsmHot::new();
        assert_eq!(t.insert(b"key", 1), None);
        assert_eq!(t.insert(b"key", 2), Some(1));
        assert_eq!(t.get(b"key"), Some(2));
    }
    
    #[test]
    fn test_many() {
        let mut t = LsmHot::with_buffer_limit(100);
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }
        assert_eq!(t.len(), 1000);
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            assert_eq!(t.get(key.as_bytes()), Some(i), "Failed at {}", i);
        }
    }
    
    #[test]
    fn test_compaction() {
        let mut t = LsmHot::with_buffer_limit(10);
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }
        // Should have compacted multiple times
        assert!(t.frozen.is_some());
        
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            assert_eq!(t.get(key.as_bytes()), Some(i), "Failed at {}", i);
        }
    }
}
