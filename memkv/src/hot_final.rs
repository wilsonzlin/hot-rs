//! HOT: Height Optimized Trie Index
//!
//! Based on "HOT: A Height Optimized Trie Index for Main-Memory Database Systems"
//! Binna et al., SIGMOD 2018
//! Reference: https://github.com/speedskater/hot
//!
//! Memory-optimized implementation:
//! - Keys stored in contiguous buffer (no per-key allocation)
//! - Leaves store (key_offset, key_len, value) = 14 bytes
//! - Nodes use sparse partial keys for compact representation

/// Child pointer (4 bytes) - high bit distinguishes leaf vs node
#[derive(Clone, Copy, Debug)]
struct Ptr(u32);

impl Ptr {
    const LEAF_BIT: u32 = 0x8000_0000;
    
    #[inline(always)]
    fn leaf(idx: u32) -> Self { Self(idx | Self::LEAF_BIT) }
    
    #[inline(always)]
    fn node(off: u32) -> Self { Self(off) }
    
    #[inline(always)]
    fn is_leaf(self) -> bool { self.0 & Self::LEAF_BIT != 0 }
    
    #[inline(always)]
    fn leaf_idx(self) -> u32 { self.0 & !Self::LEAF_BIT }
    
    #[inline(always)]
    fn node_off(self) -> u32 { self.0 }
}

/// Leaf: offset into key buffer + value
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Leaf {
    key_off: u32,  // Offset in key_data
    key_len: u16,  // Length of key
    value: u64,    // The value
}

/// HOT index
pub struct HOT {
    // All keys stored contiguously
    key_data: Vec<u8>,
    // Leaf entries (key reference + value)
    leaves: Vec<Leaf>,
    // Node arena
    nodes: Vec<u8>,
    // Root: 0 = empty, otherwise Ptr
    root: Ptr,
}

impl HOT {
    pub fn new() -> Self {
        Self {
            key_data: Vec::with_capacity(1024 * 1024),
            leaves: Vec::with_capacity(1024),
            nodes: Vec::with_capacity(64 * 1024),
            root: Ptr(0),
        }
    }

    #[inline]
    pub fn len(&self) -> usize { self.leaves.len() }
    
    #[inline]
    pub fn is_empty(&self) -> bool { self.leaves.is_empty() }

    // Store a key, return (offset, len)
    fn store_key(&mut self, key: &[u8]) -> (u32, u16) {
        let off = self.key_data.len() as u32;
        self.key_data.extend_from_slice(key);
        (off, key.len() as u16)
    }

    // Get key from leaf
    fn get_key(&self, leaf: &Leaf) -> &[u8] {
        let start = leaf.key_off as usize;
        let end = start + leaf.key_len as usize;
        &self.key_data[start..end]
    }

    // Allocate node space
    fn alloc_node(&mut self, size: usize) -> u32 {
        let off = self.nodes.len() as u32;
        self.nodes.resize(self.nodes.len() + size, 0);
        off
    }

    // Node read/write
    #[inline]
    fn n_r8(&self, off: u32) -> u8 { self.nodes[off as usize] }
    #[inline]
    fn n_w8(&mut self, off: u32, v: u8) { self.nodes[off as usize] = v; }
    #[inline]
    fn n_r16(&self, off: u32) -> u16 {
        let o = off as usize;
        u16::from_le_bytes([self.nodes[o], self.nodes[o + 1]])
    }
    #[inline]
    fn n_w16(&mut self, off: u32, v: u16) {
        let o = off as usize;
        let b = v.to_le_bytes();
        self.nodes[o] = b[0];
        self.nodes[o + 1] = b[1];
    }
    #[inline]
    fn n_r32(&self, off: u32) -> u32 {
        let o = off as usize;
        u32::from_le_bytes([self.nodes[o], self.nodes[o+1], self.nodes[o+2], self.nodes[o+3]])
    }
    #[inline]
    fn n_w32(&mut self, off: u32, v: u32) {
        let o = off as usize;
        self.nodes[o..o+4].copy_from_slice(&v.to_le_bytes());
    }

    // Get bit at position from key
    #[inline]
    fn bit_at(key: &[u8], pos: u16) -> u8 {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);
        if byte_idx < key.len() { (key[byte_idx] >> bit_idx) & 1 } else { 0 }
    }

    // Find first differing bit
    fn diff_bit(a: &[u8], b: &[u8]) -> Option<u16> {
        let max = a.len().max(b.len());
        for i in 0..max {
            let ab = a.get(i).copied().unwrap_or(0);
            let bb = b.get(i).copied().unwrap_or(0);
            if ab != bb {
                let xor = ab ^ bb;
                let bit = 7 - xor.leading_zeros() as u16;
                return Some((i as u16) * 8 + (7 - bit));
            }
        }
        None
    }

    /// Node layout (compact):
    /// [num_entries:1][num_bits:1][disc_bits:2*num_bits][partial_keys:num_entries][children:4*num_entries]
    /// 
    /// For num_bits <= 8: partial_keys are u8
    /// Total overhead per entry: 1 byte partial key + 4 bytes child ptr = 5 bytes
    /// Plus amortized: disc_bits / num_entries

    fn create_binode(&mut self, bit_pos: u16, left: Ptr, right: Ptr) -> u32 {
        // 2 entries, 1 bit: [2][1][bit_pos:2][pk0:1][pk1:1][ptr0:4][ptr1:4] = 14 bytes
        let off = self.alloc_node(14);
        self.n_w8(off, 2);      // num_entries
        self.n_w8(off + 1, 1);  // num_bits
        self.n_w16(off + 2, bit_pos);
        self.n_w8(off + 4, 0);  // left partial key = 0
        self.n_w8(off + 5, 1);  // right partial key = 1
        self.n_w32(off + 6, left.0);
        self.n_w32(off + 10, right.0);
        off
    }

    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        if self.leaves.is_empty() {
            // First entry
            let (off, len) = self.store_key(key);
            self.leaves.push(Leaf { key_off: off, key_len: len, value });
            self.root = Ptr::leaf(0);
            return None;
        }

        if self.root.is_leaf() && self.leaves.len() == 1 {
            // Single leaf - check for match or split
            let existing = self.get_key(&self.leaves[0]).to_vec();
            
            if existing.as_slice() == key {
                let old = self.leaves[0].value;
                self.leaves[0].value = value;
                return Some(old);
            }

            if let Some(diff) = Self::diff_bit(&existing, key) {
                let (off, len) = self.store_key(key);
                let new_idx = self.leaves.len() as u32;
                self.leaves.push(Leaf { key_off: off, key_len: len, value });

                let (left, right) = if Self::bit_at(&existing, diff) == 0 {
                    (Ptr::leaf(0), Ptr::leaf(new_idx))
                } else {
                    (Ptr::leaf(new_idx), Ptr::leaf(0))
                };

                let node = self.create_binode(diff, left, right);
                self.root = Ptr::node(node);
            }
            return None;
        }

        // Insert into tree
        match self.insert_rec(self.root, key, value) {
            InsertResult::Done(old) => old,
            InsertResult::NewRoot(ptr) => {
                self.root = ptr;
                None
            }
        }
    }

    fn insert_rec(&mut self, ptr: Ptr, key: &[u8], value: u64) -> InsertResult {
        if ptr.is_leaf() {
            let idx = ptr.leaf_idx() as usize;
            let existing = self.get_key(&self.leaves[idx]).to_vec();

            if existing.as_slice() == key {
                let old = self.leaves[idx].value;
                self.leaves[idx].value = value;
                return InsertResult::Done(Some(old));
            }

            if let Some(diff) = Self::diff_bit(&existing, key) {
                let (off, len) = self.store_key(key);
                let new_idx = self.leaves.len() as u32;
                self.leaves.push(Leaf { key_off: off, key_len: len, value });

                let (left, right) = if Self::bit_at(&existing, diff) == 0 {
                    (ptr, Ptr::leaf(new_idx))
                } else {
                    (Ptr::leaf(new_idx), ptr)
                };

                let node = self.create_binode(diff, left, right);
                return InsertResult::NewRoot(Ptr::node(node));
            }
            InsertResult::Done(None)
        } else {
            let node_off = ptr.node_off();
            let num_entries = self.n_r8(node_off) as usize;
            let num_bits = self.n_r8(node_off + 1) as usize;

            // Extract partial key from search key
            let disc_off = node_off + 2;
            let mut search_pk = 0u8;
            for i in 0..num_bits {
                let bit_pos = self.n_r16(disc_off + (i as u32) * 2);
                search_pk |= Self::bit_at(key, bit_pos) << i;
            }

            // Find matching child (sparse partial key is subset of search key)
            let pk_off = disc_off + (num_bits as u32) * 2;
            let ch_off = pk_off + num_entries as u32;

            let mut match_idx = 0;
            for i in 0..num_entries {
                let sparse_pk = self.n_r8(pk_off + i as u32);
                if (search_pk & sparse_pk) == sparse_pk {
                    match_idx = i;
                }
            }

            let child = Ptr(self.n_r32(ch_off + (match_idx as u32) * 4));

            match self.insert_rec(child, key, value) {
                InsertResult::Done(old) => InsertResult::Done(old),
                InsertResult::NewRoot(new_child) => {
                    // Update child pointer in place (since we're using arena)
                    self.n_w32(ch_off + (match_idx as u32) * 4, new_child.0);
                    InsertResult::Done(None)
                }
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.leaves.is_empty() { return None; }
        self.get_rec(self.root, key)
    }

    fn get_rec(&self, ptr: Ptr, key: &[u8]) -> Option<u64> {
        if ptr.is_leaf() {
            let idx = ptr.leaf_idx() as usize;
            let leaf = &self.leaves[idx];
            if self.get_key(leaf) == key {
                Some(leaf.value)
            } else {
                None
            }
        } else {
            let node_off = ptr.node_off();
            let num_entries = self.n_r8(node_off) as usize;
            let num_bits = self.n_r8(node_off + 1) as usize;

            let disc_off = node_off + 2;
            let mut search_pk = 0u8;
            for i in 0..num_bits {
                let bit_pos = self.n_r16(disc_off + (i as u32) * 2);
                search_pk |= Self::bit_at(key, bit_pos) << i;
            }

            let pk_off = disc_off + (num_bits as u32) * 2;
            let ch_off = pk_off + num_entries as u32;

            let mut match_idx = 0;
            for i in 0..num_entries {
                let sparse_pk = self.n_r8(pk_off + i as u32);
                if (search_pk & sparse_pk) == sparse_pk {
                    match_idx = i;
                }
            }

            let child = Ptr(self.n_r32(ch_off + (match_idx as u32) * 4));
            self.get_rec(child, key)
        }
    }

    pub fn memory_usage(&self) -> usize {
        self.key_data.capacity() + 
        self.leaves.capacity() * std::mem::size_of::<Leaf>() +
        self.nodes.capacity()
    }
}

impl Default for HOT {
    fn default() -> Self { Self::new() }
}

enum InsertResult {
    Done(Option<u64>),
    NewRoot(Ptr),
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
}
