//! HOT with inline values - targeting minimum overhead
//!
//! Key insight: Store values directly in key_data alongside keys
//! This eliminates the separate leaves array overhead.
//!
//! Layout: key_data = [...[len:2][key bytes][value:8]...]
//! Overhead per key: 2 (len) + tree structure
//! With compound nodes: ~14 B/K total (using 6-byte pointers for large datasets)

/// Pointer: 6 bytes (48-bit), high bit = leaf flag
/// Leaf: points directly into key_data
/// Node: points into nodes array
///
/// We use u64 internally but only store/use 48 bits (6 bytes).
/// This allows addressing up to 128TB (2^47 bytes) which is sufficient for any dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ptr(u64);

impl Ptr {
    /// High bit of 48-bit value (bit 47)
    const LEAF_BIT: u64 = 0x0000_8000_0000_0000;
    /// Mask for the offset portion (lower 47 bits)
    const OFFSET_MASK: u64 = 0x0000_7FFF_FFFF_FFFF;
    /// Maximum addressable offset (2^47 - 1 = 128TB)
    const MAX_OFFSET: u64 = 0x0000_7FFF_FFFF_FFFF;
    /// Null pointer (all 48 bits set)
    const NULL: Ptr = Ptr(0x0000_FFFF_FFFF_FFFF);

    #[inline] fn leaf(off: u64) -> Self {
        debug_assert!(off <= Self::MAX_OFFSET, "leaf offset exceeds 47-bit limit");
        Self(off | Self::LEAF_BIT)
    }
    #[inline] fn node(off: u64) -> Self {
        debug_assert!(off <= Self::MAX_OFFSET, "node offset exceeds 47-bit limit");
        Self(off)
    }
    #[inline] fn is_null(self) -> bool { self.0 == Self::NULL.0 }
    #[inline] fn is_leaf(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT != 0) }
    #[inline] #[allow(dead_code)] fn is_node(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT == 0) }
    #[inline] fn leaf_off(self) -> u64 { self.0 & Self::OFFSET_MASK }
    #[inline] fn node_off(self) -> u64 { self.0 & Self::OFFSET_MASK }

    /// Convert to 6-byte representation for storage
    #[inline] fn to_bytes(self) -> [u8; 6] {
        let bytes = self.0.to_le_bytes();
        [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]]
    }

    /// Create from 6-byte representation
    #[inline] fn from_bytes(bytes: [u8; 6]) -> Self {
        let val = u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], 0, 0]);
        Self(val)
    }
}

/// BiNode size: [bit_pos:2][left:6][right:6] = 14 bytes
const BINODE_SIZE: usize = 14;

pub struct InlineHot {
    /// Keys + values stored inline: [len:2][key][value:8]...
    key_data: Vec<u8>,
    /// Internal nodes
    nodes: Vec<u8>,
    /// Root pointer
    root: Ptr,
    /// Number of entries
    count: usize,
}

impl InlineHot {
    pub fn new() -> Self {
        Self {
            key_data: Vec::new(),
            nodes: Vec::new(),
            root: Ptr::NULL,
            count: 0,
        }
    }

    #[inline] pub fn len(&self) -> usize { self.count }
    #[inline] pub fn is_empty(&self) -> bool { self.count == 0 }

    fn store_entry(&mut self, key: &[u8], value: u64) -> u64 {
        let off = self.key_data.len();
        assert!(off as u64 <= Ptr::MAX_OFFSET - key.len() as u64 - 10,
            "InlineHot: key_data exceeds 47-bit (128TB) addressable space");
        let len = key.len() as u16;
        self.key_data.extend_from_slice(&len.to_le_bytes());
        self.key_data.extend_from_slice(key);
        self.key_data.extend_from_slice(&value.to_le_bytes());
        off as u64
    }

    fn get_key(&self, off: u64) -> &[u8] {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        &self.key_data[o + 2..o + 2 + len]
    }

    fn get_value(&self, off: u64) -> u64 {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        let val_off = o + 2 + len;
        u64::from_le_bytes([
            self.key_data[val_off], self.key_data[val_off + 1],
            self.key_data[val_off + 2], self.key_data[val_off + 3],
            self.key_data[val_off + 4], self.key_data[val_off + 5],
            self.key_data[val_off + 6], self.key_data[val_off + 7],
        ])
    }

    fn set_value(&mut self, off: u64, value: u64) {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        let val_off = o + 2 + len;
        self.key_data[val_off..val_off + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn alloc_binode(&mut self) -> u64 {
        let off = self.nodes.len();
        assert!((off as u64) <= Ptr::MAX_OFFSET - BINODE_SIZE as u64,
            "InlineHot: nodes exceeds 47-bit (128TB) addressable space");
        self.nodes.resize(off + BINODE_SIZE, 0);
        off as u64
    }

    fn r16(&self, o: u64) -> u16 {
        let i = o as usize;
        u16::from_le_bytes([self.nodes[i], self.nodes[i + 1]])
    }
    fn w16(&mut self, o: u64, v: u16) {
        let i = o as usize;
        let b = v.to_le_bytes();
        self.nodes[i] = b[0];
        self.nodes[i + 1] = b[1];
    }

    /// Read a 6-byte pointer from nodes at offset o
    fn r48(&self, o: u64) -> Ptr {
        let i = o as usize;
        Ptr::from_bytes([
            self.nodes[i], self.nodes[i+1], self.nodes[i+2],
            self.nodes[i+3], self.nodes[i+4], self.nodes[i+5]
        ])
    }

    /// Write a 6-byte pointer to nodes at offset o
    fn w48(&mut self, o: u64, ptr: Ptr) {
        let i = o as usize;
        let bytes = ptr.to_bytes();
        self.nodes[i..i+6].copy_from_slice(&bytes);
    }

    fn binode_bit(&self, off: u64) -> u16 { self.r16(off) }
    fn binode_left(&self, off: u64) -> Ptr { self.r48(off + 2) }
    fn binode_right(&self, off: u64) -> Ptr { self.r48(off + 8) }
    fn set_binode(&mut self, off: u64, bit: u16, left: Ptr, right: Ptr) {
        self.w16(off, bit);
        self.w48(off + 2, left);
        self.w48(off + 8, right);
    }
    fn set_binode_left(&mut self, off: u64, left: Ptr) { self.w48(off + 2, left); }
    fn set_binode_right(&mut self, off: u64, right: Ptr) { self.w48(off + 8, right); }

    #[inline]
    fn bit_at(key: &[u8], pos: u16) -> u8 {
        let byte = (pos / 8) as usize;
        let bit = 7 - (pos % 8);
        if byte < key.len() { (key[byte] >> bit) & 1 } else { 0 }
    }

    fn first_diff_bit(a: &[u8], b: &[u8]) -> Option<u16> {
        let max = a.len().max(b.len());
        for i in 0..max {
            let ab = a.get(i).copied().unwrap_or(0);
            let bb = b.get(i).copied().unwrap_or(0);
            if ab != bb {
                let xor = ab ^ bb;
                let leading = xor.leading_zeros();
                return Some(i as u16 * 8 + leading as u16);
            }
        }
        None
    }

    fn create_leaf(&mut self, key: &[u8], value: u64) -> Ptr {
        let off = self.store_entry(key, value);
        self.count += 1;
        Ptr::leaf(off)
    }

    fn create_binode(&mut self, bit: u16, left: Ptr, right: Ptr) -> Ptr {
        let off = self.alloc_binode();
        self.set_binode(off, bit, left, right);
        Ptr::node(off)
    }

    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        if self.root.is_null() {
            self.root = self.create_leaf(key, value);
            return None;
        }

        match self.insert_into(self.root, key, value) {
            InsertResult::Updated(old) => Some(old),
            InsertResult::Replaced(new_ptr) => {
                self.root = new_ptr;
                None
            }
            InsertResult::NoChange => None,
        }
    }

    fn insert_into(&mut self, ptr: Ptr, key: &[u8], value: u64) -> InsertResult {
        if ptr.is_leaf() {
            let existing = self.get_key(ptr.leaf_off()).to_vec();

            if existing == key {
                let old = self.get_value(ptr.leaf_off());
                self.set_value(ptr.leaf_off(), value);
                return InsertResult::Updated(old);
            }

            if let Some(diff) = Self::first_diff_bit(&existing, key) {
                let new_leaf = self.create_leaf(key, value);
                let ex_bit = Self::bit_at(&existing, diff);
                let (left, right) = if ex_bit == 0 {
                    (ptr, new_leaf)
                } else {
                    (new_leaf, ptr)
                };
                let binode = self.create_binode(diff, left, right);
                return InsertResult::Replaced(binode);
            }
            InsertResult::NoChange
        } else {
            let off = ptr.node_off();
            let bit = self.binode_bit(off);
            let key_bit = Self::bit_at(key, bit);

            if key_bit == 0 {
                let left = self.binode_left(off);
                match self.insert_into(left, key, value) {
                    InsertResult::Updated(old) => InsertResult::Updated(old),
                    InsertResult::Replaced(new_left) => {
                        self.set_binode_left(off, new_left);
                        InsertResult::NoChange
                    }
                    InsertResult::NoChange => InsertResult::NoChange,
                }
            } else {
                let right = self.binode_right(off);
                match self.insert_into(right, key, value) {
                    InsertResult::Updated(old) => InsertResult::Updated(old),
                    InsertResult::Replaced(new_right) => {
                        self.set_binode_right(off, new_right);
                        InsertResult::NoChange
                    }
                    InsertResult::NoChange => InsertResult::NoChange,
                }
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root.is_null() { return None; }
        self.get_from(self.root, key)
    }

    fn get_from(&self, ptr: Ptr, key: &[u8]) -> Option<u64> {
        if ptr.is_leaf() {
            if self.get_key(ptr.leaf_off()) == key {
                Some(self.get_value(ptr.leaf_off()))
            } else {
                None
            }
        } else {
            let off = ptr.node_off();
            let bit = self.binode_bit(off);
            if Self::bit_at(key, bit) == 0 {
                self.get_from(self.binode_left(off), key)
            } else {
                self.get_from(self.binode_right(off), key)
            }
        }
    }

    pub fn memory_usage(&self) -> usize {
        self.key_data.capacity() + self.nodes.capacity()
    }

    pub fn memory_usage_actual(&self) -> usize {
        self.key_data.len() + self.nodes.len()
    }

    pub fn shrink_to_fit(&mut self) {
        self.key_data.shrink_to_fit();
        self.nodes.shrink_to_fit();
    }
}

impl Default for InlineHot {
    fn default() -> Self { Self::new() }
}

enum InsertResult {
    Updated(u64),
    Replaced(Ptr),
    NoChange,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let mut t = InlineHot::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        assert_eq!(t.get(b"hello"), Some(1));
        assert_eq!(t.get(b"world"), Some(2));
        assert_eq!(t.get(b"missing"), None);
    }

    #[test]
    fn test_update() {
        let mut t = InlineHot::new();
        assert_eq!(t.insert(b"key", 1), None);
        assert_eq!(t.insert(b"key", 2), Some(1));
        assert_eq!(t.get(b"key"), Some(2));
    }

    #[test]
    fn test_many() {
        let mut t = InlineHot::new();
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
    fn test_ptr_roundtrip() {
        // Test leaf pointer roundtrip
        let leaf = Ptr::leaf(0x123456789ABC);
        let bytes = leaf.to_bytes();
        let restored = Ptr::from_bytes(bytes);
        assert_eq!(leaf, restored);
        assert!(restored.is_leaf());
        assert_eq!(restored.leaf_off(), 0x123456789ABC);

        // Test node pointer roundtrip
        let node = Ptr::node(0x7FFF_FFFF_FFFF);
        let bytes = node.to_bytes();
        let restored = Ptr::from_bytes(bytes);
        assert_eq!(node, restored);
        assert!(restored.is_node());
        assert_eq!(restored.node_off(), 0x7FFF_FFFF_FFFF);

        // Test null pointer
        let null = Ptr::NULL;
        let bytes = null.to_bytes();
        let restored = Ptr::from_bytes(bytes);
        assert!(restored.is_null());
    }
}
