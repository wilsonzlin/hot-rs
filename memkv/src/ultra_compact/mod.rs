//! UltraCompact: Pushing for absolute minimum overhead
//!
//! Theoretical minimum for sorted array:
//! - Key bytes: N (stored once)
//! - Per-key overhead: 
//!   - Value: 8 bytes (u64) or 4 bytes (u32)
//!   - Key length: 1-2 bytes (varint)
//!   - Offset: 1-4 bytes (varint or delta)
//!
//! Target: 10-12 bytes overhead per key

/// Varint encoding (1-5 bytes for u32)
#[inline]
fn encode_varint(mut value: u32, buf: &mut Vec<u8>) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

#[inline]
fn decode_varint(data: &[u8], pos: &mut usize) -> u32 {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        let byte = data[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u32) << shift;
        if byte < 0x80 {
            break;
        }
        shift += 7;
    }
    result
}

/// UltraCompact: Minimal overhead sorted store
/// 
/// Layout:
/// - data: [entry0][entry1]... where entry = [key_len:varint][key bytes][value:u64]
/// - offsets: [offset0:u32][offset1:u32]...
/// 
/// For 10M keys with avg 51.7 byte keys:
/// - Key data: 517 MB
/// - Per-key overhead: 10 bytes (8 value + 1 len + 1 offset amortized via delta)
pub struct UltraCompact {
    // Key data: [key_len:varint][key bytes][value:8 bytes]
    data: Vec<u8>,
    // Offsets into data (could use delta encoding)
    offsets: Vec<u32>,
    len: usize,
}

impl UltraCompact {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            offsets: Vec::new(),
            len: 0,
        }
    }
    
    pub fn with_capacity(keys: usize, total_key_bytes: usize) -> Self {
        // Estimate: 1 byte for varint len + key bytes + 8 bytes value
        let data_cap = total_key_bytes + keys * 9;
        Self {
            data: Vec::with_capacity(data_cap),
            offsets: Vec::with_capacity(keys),
            len: 0,
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    #[inline]
    fn get_entry(&self, idx: usize) -> (&[u8], u64) {
        let off = self.offsets[idx] as usize;
        let mut pos = off;
        let key_len = decode_varint(&self.data, &mut pos) as usize;
        let key = &self.data[pos..pos + key_len];
        pos += key_len;
        let value = u64::from_le_bytes(self.data[pos..pos+8].try_into().unwrap());
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
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.len == 0 { return None; }
        match self.binary_search(key) {
            Ok(idx) => Some(self.get_entry(idx).1),
            Err(_) => None,
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        match self.binary_search(key) {
            Ok(idx) => {
                // Update value in place
                let off = self.offsets[idx] as usize;
                let mut pos = off;
                let key_len = decode_varint(&self.data, &mut pos) as usize;
                pos += key_len;
                let old = u64::from_le_bytes(self.data[pos..pos+8].try_into().unwrap());
                self.data[pos..pos+8].copy_from_slice(&value.to_le_bytes());
                Some(old)
            }
            Err(idx) => {
                // Insert new entry
                let entry_off = self.data.len() as u32;
                
                // Write entry: [key_len:varint][key][value:8]
                encode_varint(key.len() as u32, &mut self.data);
                self.data.extend_from_slice(key);
                self.data.extend_from_slice(&value.to_le_bytes());
                
                // Insert offset
                self.offsets.insert(idx, entry_off);
                self.len += 1;
                None
            }
        }
    }
    
    pub fn memory_stats(&self) -> UltraCompactStats {
        let data_bytes = self.data.capacity();
        let offsets_bytes = self.offsets.capacity() * 4;
        let total = data_bytes + offsets_bytes;
        
        // Calculate raw key bytes
        let mut raw_key_bytes = 0;
        for i in 0..self.len {
            let (key, _) = self.get_entry(i);
            raw_key_bytes += key.len();
        }
        
        UltraCompactStats {
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

impl Default for UltraCompact {
    fn default() -> Self { Self::new() }
}

pub struct UltraCompactStats {
    pub data_bytes: usize,
    pub offsets_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
}

/// DeltaCompact: Even smaller using delta-encoded offsets
/// 
/// Instead of storing absolute offsets (4 bytes each),
/// store deltas from previous offset (1-2 bytes with varint)
pub struct DeltaCompact {
    // Key data: [key_len:varint][key bytes][value:8 bytes]
    data: Vec<u8>,
    // Delta-encoded offsets (first is absolute, rest are deltas)
    deltas: Vec<u8>,
    len: usize,
}

impl DeltaCompact {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            deltas: Vec::new(),
            len: 0,
        }
    }
    
    pub fn with_capacity(keys: usize, total_key_bytes: usize) -> Self {
        let data_cap = total_key_bytes + keys * 9;
        // Estimate 2 bytes per delta on average
        let delta_cap = keys * 2;
        Self {
            data: Vec::with_capacity(data_cap),
            deltas: Vec::with_capacity(delta_cap),
            len: 0,
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    // Build offset index (for random access)
    fn get_offset(&self, idx: usize) -> usize {
        let mut pos = 0;
        let mut offset = 0u32;
        for i in 0..=idx {
            let delta = decode_varint(&self.deltas, &mut pos);
            if i == 0 {
                offset = delta;
            } else {
                offset += delta;
            }
        }
        offset as usize
    }
    
    #[inline]
    fn get_entry_at_offset(&self, off: usize) -> (&[u8], u64) {
        let mut pos = off;
        let key_len = decode_varint(&self.data, &mut pos) as usize;
        let key = &self.data[pos..pos + key_len];
        pos += key_len;
        let value = u64::from_le_bytes(self.data[pos..pos+8].try_into().unwrap());
        (key, value)
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.len == 0 { return None; }
        
        // Linear scan with delta decoding (fast for sequential access)
        let mut delta_pos = 0;
        let mut offset = 0u32;
        
        for i in 0..self.len {
            let delta = decode_varint(&self.deltas, &mut delta_pos);
            if i == 0 {
                offset = delta;
            } else {
                offset += delta;
            }
            
            let (k, v) = self.get_entry_at_offset(offset as usize);
            match k.cmp(key) {
                std::cmp::Ordering::Equal => return Some(v),
                std::cmp::Ordering::Greater => return None, // Sorted, so won't find it
                std::cmp::Ordering::Less => continue,
            }
        }
        None
    }
    
    /// Bulk insert (must be sorted!)
    pub fn insert_sorted(&mut self, key: &[u8], value: u64) {
        let entry_off = self.data.len() as u32;
        
        // Compute delta from previous offset
        let delta = if self.len == 0 {
            entry_off
        } else {
            let prev_off = self.get_offset(self.len - 1) as u32;
            entry_off - prev_off
        };
        
        // Write entry
        encode_varint(key.len() as u32, &mut self.data);
        self.data.extend_from_slice(key);
        self.data.extend_from_slice(&value.to_le_bytes());
        
        // Write delta
        encode_varint(delta, &mut self.deltas);
        self.len += 1;
    }
    
    pub fn memory_stats(&self) -> DeltaCompactStats {
        let data_bytes = self.data.capacity();
        let deltas_bytes = self.deltas.capacity();
        let total = data_bytes + deltas_bytes;
        
        let mut raw_key_bytes = 0;
        let mut delta_pos = 0;
        let mut offset = 0u32;
        for i in 0..self.len {
            let delta = decode_varint(&self.deltas, &mut delta_pos);
            if i == 0 { offset = delta; } else { offset += delta; }
            let (key, _) = self.get_entry_at_offset(offset as usize);
            raw_key_bytes += key.len();
        }
        
        DeltaCompactStats {
            data_bytes,
            deltas_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: total.saturating_sub(raw_key_bytes),
            overhead_per_key: if self.len > 0 {
                (total as f64 - raw_key_bytes as f64) / self.len as f64
            } else { 0.0 },
        }
    }
}

impl Default for DeltaCompact {
    fn default() -> Self { Self::new() }
}

pub struct DeltaCompactStats {
    pub data_bytes: usize,
    pub deltas_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ultra_compact() {
        let mut store = UltraCompact::new();
        
        store.insert(b"apple", 1);
        store.insert(b"banana", 2);
        store.insert(b"cherry", 3);
        
        assert_eq!(store.get(b"apple"), Some(1));
        assert_eq!(store.get(b"banana"), Some(2));
        assert_eq!(store.get(b"cherry"), Some(3));
        assert_eq!(store.get(b"date"), None);
    }
    
    #[test]
    fn test_ultra_compact_update() {
        let mut store = UltraCompact::new();
        
        assert_eq!(store.insert(b"key", 1), None);
        assert_eq!(store.insert(b"key", 2), Some(1));
        assert_eq!(store.get(b"key"), Some(2));
    }
    
    #[test]
    fn test_delta_compact() {
        let mut store = DeltaCompact::new();
        
        // Must insert in sorted order!
        store.insert_sorted(b"apple", 1);
        store.insert_sorted(b"banana", 2);
        store.insert_sorted(b"cherry", 3);
        
        assert_eq!(store.get(b"apple"), Some(1));
        assert_eq!(store.get(b"banana"), Some(2));
        assert_eq!(store.get(b"cherry"), Some(3));
        assert_eq!(store.get(b"date"), None);
    }
    
    #[test]
    fn test_varint() {
        let mut buf = Vec::new();
        
        encode_varint(0, &mut buf);
        encode_varint(127, &mut buf);
        encode_varint(128, &mut buf);
        encode_varint(16383, &mut buf);
        encode_varint(16384, &mut buf);
        
        let mut pos = 0;
        assert_eq!(decode_varint(&buf, &mut pos), 0);
        assert_eq!(decode_varint(&buf, &mut pos), 127);
        assert_eq!(decode_varint(&buf, &mut pos), 128);
        assert_eq!(decode_varint(&buf, &mut pos), 16383);
        assert_eq!(decode_varint(&buf, &mut pos), 16384);
    }
}
