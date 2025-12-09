//! HOT: Height Optimized Trie - Simplified Implementation
//!
//! This implementation focuses on correctness first.
//! Based on HOT paper concepts but with simpler node management.
//! Uses 6-byte (48-bit) pointers to support datasets up to 128TB.

/// Child pointer - 6 bytes (48-bit), high bit distinguishes leaf vs node
///
/// We use u64 internally but only store/use 48 bits (6 bytes).
/// This allows addressing up to 128TB (2^47 bytes) which is sufficient for any dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ptr(u64);

impl Ptr {
    /// High bit of 48-bit value (bit 47)
    const LEAF_BIT: u64 = 0x0000_8000_0000_0000;
    /// Mask for the offset/index portion (lower 47 bits)
    const OFFSET_MASK: u64 = 0x0000_7FFF_FFFF_FFFF;
    /// Maximum addressable offset (2^47 - 1 = 128TB)
    const MAX_OFFSET: u64 = 0x0000_7FFF_FFFF_FFFF;
    /// Null pointer (all 48 bits set)
    const NULL: Ptr = Ptr(0x0000_FFFF_FFFF_FFFF);

    #[inline] fn leaf(idx: u64) -> Self {
        debug_assert!(idx <= Self::MAX_OFFSET, "leaf index exceeds 47-bit limit");
        Self(idx | Self::LEAF_BIT)
    }
    #[inline] fn node(off: u64) -> Self {
        debug_assert!(off <= Self::MAX_OFFSET, "node offset exceeds 47-bit limit");
        Self(off)
    }
    #[inline] fn is_null(self) -> bool { self.0 == Self::NULL.0 }
    #[inline] fn is_leaf(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT != 0) }
    #[inline] #[allow(dead_code)] fn is_node(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT == 0) }
    #[inline] fn leaf_idx(self) -> u64 { self.0 & Self::OFFSET_MASK }
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

/// BiNode: binary node with single discriminator bit
/// Layout: [bit_pos:2][left:6][right:6] = 14 bytes
const BINODE_SIZE: usize = 14;

/// Leaf entry - uses u64 for offsets to support large datasets
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct Leaf {
    key_off: u64,  // Changed from u32 to u64 for large datasets
    key_len: u16,
    value: u64,
}

pub struct HOT {
    key_data: Vec<u8>,
    leaves: Vec<Leaf>,
    nodes: Vec<u8>,
    root: Ptr,
}

impl HOT {
    pub fn new() -> Self {
        Self {
            key_data: Vec::new(),
            leaves: Vec::new(),
            nodes: Vec::new(),
            root: Ptr::NULL,
        }
    }

    pub fn with_capacity(key_count: usize, avg_key_len: usize) -> Self {
        Self {
            key_data: Vec::with_capacity(key_count * avg_key_len),
            leaves: Vec::with_capacity(key_count),
            nodes: Vec::with_capacity(key_count * BINODE_SIZE),
            root: Ptr::NULL,
        }
    }

    #[inline] pub fn len(&self) -> usize { self.leaves.len() }
    #[inline] pub fn is_empty(&self) -> bool { self.leaves.is_empty() }

    fn store_key(&mut self, key: &[u8]) -> (u64, u16) {
        let off = self.key_data.len();
        assert!((off as u64) <= Ptr::MAX_OFFSET - key.len() as u64,
            "HOT: key_data exceeds 47-bit (128TB) addressable space");
        self.key_data.extend_from_slice(key);
        (off as u64, key.len() as u16)
    }

    fn get_key(&self, leaf: &Leaf) -> &[u8] {
        let s = leaf.key_off as usize;
        let e = s + leaf.key_len as usize;
        &self.key_data[s..e]
    }

    fn alloc_binode(&mut self) -> u64 {
        let off = self.nodes.len();
        assert!((off as u64) <= Ptr::MAX_OFFSET - BINODE_SIZE as u64,
            "HOT: nodes exceeds 47-bit (128TB) addressable space");
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
        let (off, len) = self.store_key(key);
        let idx = self.leaves.len();
        assert!((idx as u64) <= Ptr::MAX_OFFSET, "HOT: leaves exceeds 47-bit addressable space");
        self.leaves.push(Leaf { key_off: off, key_len: len, value });
        Ptr::leaf(idx as u64)
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
            let idx = ptr.leaf_idx() as usize;
            let existing = self.get_key(&self.leaves[idx]).to_vec();

            if existing == key {
                let old = self.leaves[idx].value;
                self.leaves[idx].value = value;
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
        if self.root.is_null() {
            return None;
        }
        self.get_from(self.root, key)
    }

    fn get_from(&self, ptr: Ptr, key: &[u8]) -> Option<u64> {
        if ptr.is_leaf() {
            let leaf = &self.leaves[ptr.leaf_idx() as usize];
            if self.get_key(leaf) == key {
                Some(leaf.value)
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
        self.key_data.capacity() +
        self.leaves.capacity() * std::mem::size_of::<Leaf>() +
        self.nodes.capacity()
    }

    pub fn memory_usage_actual(&self) -> usize {
        self.key_data.len() +
        self.leaves.len() * std::mem::size_of::<Leaf>() +
        self.nodes.len()
    }

    /// Shrink internal allocations to fit actual data
    pub fn shrink_to_fit(&mut self) {
        self.key_data.shrink_to_fit();
        self.leaves.shrink_to_fit();
        self.nodes.shrink_to_fit();
    }
}

impl Default for HOT {
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
        let mut t = HOT::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        assert_eq!(t.get(b"hello"), Some(1));
        assert_eq!(t.get(b"world"), Some(2));
        assert_eq!(t.get(b"missing"), None);
    }

    #[test]
    fn test_update() {
        let mut t = HOT::new();
        assert_eq!(t.insert(b"key", 1), None);
        assert_eq!(t.insert(b"key", 2), Some(1));
        assert_eq!(t.get(b"key"), Some(2));
    }

    #[test]
    fn test_three() {
        let mut t = HOT::new();
        t.insert(b"aaa", 1);
        t.insert(b"bbb", 2);
        t.insert(b"ccc", 3);
        assert_eq!(t.get(b"aaa"), Some(1));
        assert_eq!(t.get(b"bbb"), Some(2));
        assert_eq!(t.get(b"ccc"), Some(3));
    }

    #[test]
    fn test_many() {
        let mut t = HOT::new();
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
    fn test_prefixes() {
        let mut t = HOT::new();
        t.insert(b"a", 1);
        t.insert(b"ab", 2);
        t.insert(b"abc", 3);
        t.insert(b"abcd", 4);
        assert_eq!(t.get(b"a"), Some(1));
        assert_eq!(t.get(b"ab"), Some(2));
        assert_eq!(t.get(b"abc"), Some(3));
        assert_eq!(t.get(b"abcd"), Some(4));
    }

    #[test]
    fn test_random_order() {
        let mut t = HOT::new();
        let keys = [5, 2, 8, 1, 9, 3, 7, 4, 6, 0];
        for &i in &keys {
            let key = format!("key{:02}", i);
            t.insert(key.as_bytes(), i as u64);
        }
        for &i in &keys {
            let key = format!("key{:02}", i);
            assert_eq!(t.get(key.as_bytes()), Some(i as u64), "Failed at {}", i);
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
        assert_eq!(restored.leaf_idx(), 0x123456789ABC);

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
