//! Minimal: Absolute minimum overhead for sorted data
//!
//! Key insight: If values are sequential indices (0, 1, 2, ...), 
//! we don't need to store them at all!
//!
//! For other cases, use the smallest value type that fits.

/// MinimalSorted: Zero value storage for sequential indices
/// 
/// Layout: Keys with length prefixes + offset index for fast lookup
/// Overhead per key: 2 bytes (len) + 4 bytes (offset) = 6 bytes
/// But value is implicit (index) so saves 8 bytes vs u64
/// Net overhead: 6 bytes (compared to 14 for GLORY)
pub struct MinimalSorted {
    // [len:u16][key bytes][len:u16][key bytes]...
    data: Vec<u8>,
    // Offsets for O(1) access to each key
    offsets: Vec<u32>,
    // Number of keys
    len: usize,
}

impl MinimalSorted {
    pub fn new() -> Self {
        Self { data: Vec::new(), offsets: Vec::new(), len: 0 }
    }
    
    pub fn with_capacity(total_key_bytes: usize, num_keys: usize) -> Self {
        Self {
            data: Vec::with_capacity(total_key_bytes + num_keys * 2),
            offsets: Vec::with_capacity(num_keys),
            len: 0,
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    /// Add key (must be in sorted order, value is implicit index)
    pub fn push(&mut self, key: &[u8]) {
        let offset = self.data.len() as u32;
        self.offsets.push(offset);
        
        let len = key.len() as u16;
        self.data.extend_from_slice(&len.to_le_bytes());
        self.data.extend_from_slice(key);
        self.len += 1;
    }
    
    /// Get key at index (O(1) with offset index)
    #[inline]
    fn get_key_at(&self, idx: usize) -> &[u8] {
        let pos = self.offsets[idx] as usize;
        let len = u16::from_le_bytes([self.data[pos], self.data[pos+1]]) as usize;
        &self.data[pos + 2..pos + 2 + len]
    }
    
    /// Binary search (returns index which IS the value)
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.len == 0 { return None; }
        
        let mut lo = 0usize;
        let mut hi = self.len;
        
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_key = self.get_key_at(mid);
            
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid as u64),
            }
        }
        None
    }
    
    pub fn memory_stats(&self) -> MinimalStats {
        let data_bytes = self.data.capacity();
        let offsets_bytes = self.offsets.capacity() * 4;
        let total = data_bytes + offsets_bytes;
        
        // Calculate raw key bytes
        let mut raw_key_bytes = 0;
        for i in 0..self.len {
            raw_key_bytes += self.get_key_at(i).len();
        }
        
        MinimalStats {
            data_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: total.saturating_sub(raw_key_bytes),
            overhead_per_key: if self.len > 0 {
                (total as f64 - raw_key_bytes as f64) / self.len as f64
            } else { 0.0 },
        }
    }
}

impl Default for MinimalSorted {
    fn default() -> Self { Self::new() }
}

pub struct MinimalStats {
    pub data_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
}

/// Compact32: Use u32 values instead of u64 (saves 4 bytes per key)
/// 
/// Overhead per key: 2 (len) + 4 (value) + 4 (offset) = 10 bytes
pub struct Compact32 {
    // [len:u16][key bytes][value:u32]
    data: Vec<u8>,
    offsets: Vec<u32>,
    len: usize,
}

impl Compact32 {
    pub fn new() -> Self {
        Self { data: Vec::new(), offsets: Vec::new(), len: 0 }
    }
    
    pub fn with_capacity(keys: usize, total_key_bytes: usize) -> Self {
        Self {
            data: Vec::with_capacity(total_key_bytes + keys * 6),
            offsets: Vec::with_capacity(keys),
            len: 0,
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    fn get_entry(&self, idx: usize) -> (&[u8], u32) {
        let off = self.offsets[idx] as usize;
        let len = u16::from_le_bytes([self.data[off], self.data[off+1]]) as usize;
        let key = &self.data[off + 2..off + 2 + len];
        let value = u32::from_le_bytes(self.data[off + 2 + len..off + 6 + len].try_into().unwrap());
        (key, value)
    }
    
    fn binary_search(&self, key: &[u8]) -> Result<usize, usize> {
        let mut lo = 0;
        let mut hi = self.len;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (k, _) = self.get_entry(mid);
            match k.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(mid),
            }
        }
        Err(lo)
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u32> {
        if self.len == 0 { return None; }
        match self.binary_search(key) {
            Ok(idx) => Some(self.get_entry(idx).1),
            Err(_) => None,
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: u32) -> Option<u32> {
        match self.binary_search(key) {
            Ok(idx) => {
                let off = self.offsets[idx] as usize;
                let len = u16::from_le_bytes([self.data[off], self.data[off+1]]) as usize;
                let val_off = off + 2 + len;
                let old = u32::from_le_bytes(self.data[val_off..val_off+4].try_into().unwrap());
                self.data[val_off..val_off+4].copy_from_slice(&value.to_le_bytes());
                Some(old)
            }
            Err(idx) => {
                let entry_off = self.data.len() as u32;
                let len = key.len() as u16;
                self.data.extend_from_slice(&len.to_le_bytes());
                self.data.extend_from_slice(key);
                self.data.extend_from_slice(&value.to_le_bytes());
                self.offsets.insert(idx, entry_off);
                self.len += 1;
                None
            }
        }
    }
    
    pub fn memory_stats(&self) -> Compact32Stats {
        let data_bytes = self.data.capacity();
        let offsets_bytes = self.offsets.capacity() * 4;
        let total = data_bytes + offsets_bytes;
        
        let mut raw_key_bytes = 0;
        for i in 0..self.len {
            let (key, _) = self.get_entry(i);
            raw_key_bytes += key.len();
        }
        
        Compact32Stats {
            data_bytes,
            offsets_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: total.saturating_sub(raw_key_bytes),
            overhead_per_key: if self.len > 0 {
                (total as f64 - raw_key_bytes as f64) / self.len as f64
            } else { 0.0 },
        }
    }
}

impl Default for Compact32 {
    fn default() -> Self { Self::new() }
}

pub struct Compact32Stats {
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
    fn test_minimal_sorted() {
        let mut store = MinimalSorted::new();
        
        // Must insert in sorted order
        store.push(b"apple");
        store.push(b"banana");
        store.push(b"cherry");
        
        assert_eq!(store.get(b"apple"), Some(0));
        assert_eq!(store.get(b"banana"), Some(1));
        assert_eq!(store.get(b"cherry"), Some(2));
        assert_eq!(store.get(b"date"), None);
    }
    
    #[test]
    fn test_compact32() {
        let mut store = Compact32::new();
        
        store.insert(b"apple", 100);
        store.insert(b"banana", 200);
        store.insert(b"cherry", 300);
        
        assert_eq!(store.get(b"apple"), Some(100));
        assert_eq!(store.get(b"banana"), Some(200));
        assert_eq!(store.get(b"cherry"), Some(300));
        assert_eq!(store.get(b"date"), None);
    }
    
    #[test]
    fn test_minimal_overhead() {
        let mut store = MinimalSorted::with_capacity(1000, 100);
        
        for i in 0..100 {
            let key = format!("key{:03}", i);
            store.push(key.as_bytes());
        }
        
        let stats = store.memory_stats();
        // Should be ~2 bytes per key (just the length prefix)
        println!("MinimalSorted overhead: {:.1} bytes/key", stats.overhead_per_key);
        assert!(stats.overhead_per_key < 3.0);
    }
}

/// VarintMinimal: Even smaller using varint for key lengths
/// 
/// Layout: [len:varint][key bytes]... + offset index
/// Overhead: 1-2 bytes (varint len) + 4 bytes (offset) = 5-6 bytes
pub struct VarintMinimal {
    data: Vec<u8>,
    offsets: Vec<u32>,
    len: usize,
}

fn encode_varint_to(mut value: u32, buf: &mut Vec<u8>) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

fn decode_varint_at(data: &[u8], pos: &mut usize) -> u32 {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u32) << shift;
        if byte < 0x80 { break; }
        shift += 7;
    }
    result
}

impl VarintMinimal {
    pub fn new() -> Self {
        Self { data: Vec::new(), offsets: Vec::new(), len: 0 }
    }
    
    pub fn with_capacity(total_key_bytes: usize, num_keys: usize) -> Self {
        Self {
            data: Vec::with_capacity(total_key_bytes + num_keys),
            offsets: Vec::with_capacity(num_keys),
            len: 0,
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    pub fn push(&mut self, key: &[u8]) {
        let offset = self.data.len() as u32;
        self.offsets.push(offset);
        encode_varint_to(key.len() as u32, &mut self.data);
        self.data.extend_from_slice(key);
        self.len += 1;
    }
    
    #[inline]
    fn get_key_at(&self, idx: usize) -> &[u8] {
        let mut pos = self.offsets[idx] as usize;
        let len = decode_varint_at(&self.data, &mut pos) as usize;
        &self.data[pos..pos + len]
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.len == 0 { return None; }
        
        let mut lo = 0usize;
        let mut hi = self.len;
        
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_key = self.get_key_at(mid);
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid as u64),
            }
        }
        None
    }
    
    pub fn memory_stats(&self) -> VarintMinimalStats {
        let data_bytes = self.data.capacity();
        let offsets_bytes = self.offsets.capacity() * 4;
        let total = data_bytes + offsets_bytes;
        
        let mut raw_key_bytes = 0;
        for i in 0..self.len {
            raw_key_bytes += self.get_key_at(i).len();
        }
        
        VarintMinimalStats {
            data_bytes,
            offsets_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: total.saturating_sub(raw_key_bytes),
            overhead_per_key: if self.len > 0 {
                (total as f64 - raw_key_bytes as f64) / self.len as f64
            } else { 0.0 },
        }
    }
}

impl Default for VarintMinimal {
    fn default() -> Self { Self::new() }
}

pub struct VarintMinimalStats {
    pub data_bytes: usize,
    pub offsets_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
}

#[test]
fn test_varint_minimal() {
    let mut store = VarintMinimal::new();
    store.push(b"apple");
    store.push(b"banana");
    store.push(b"cherry");
    assert_eq!(store.get(b"apple"), Some(0));
    assert_eq!(store.get(b"banana"), Some(1));
    assert_eq!(store.get(b"cherry"), Some(2));
}
