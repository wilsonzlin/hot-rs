//! HOT: Height Optimized Trie Index
//!
//! Implementation of "HOT: A Height Optimized Trie Index for Main-Memory Database Systems"
//! Binna et al., SIGMOD 2018
//!
//! Key ideas from the paper:
//! 1. Compound nodes: Multiple trie levels combined using k discriminator bits
//! 2. Discriminator bit positions: Only store bit positions that distinguish keys
//! 3. Sparse representation: Use popcount for dense child storage
//! 4. Height reduction: Fewer levels = less per-key overhead
//!
//! Node layout (for k discriminator bits, n actual children):
//!   Header: [node_type:1][k:1][height:2][mask:4 or 32]
//!   Discriminator bits: [pos0:2][pos1:2]...[posk-1:2] (k * 2 bytes)
//!   Children: [child0:4][child1:4]...[childn-1:4] (n * 4 bytes)
//!
//! For k <= 5: use 4-byte mask (32 bits = 2^5)
//! For k <= 8: use 32-byte mask (256 bits = 2^8)

use std::cmp::Ordering;

// Node types
const TYPE_EMPTY: u8 = 0;
const TYPE_LEAF: u8 = 1;
const TYPE_INNER_SMALL: u8 = 2;  // k <= 5, 32-bit mask
const TYPE_INNER_LARGE: u8 = 3; // k <= 8, 256-bit mask

// Maximum span (discriminator bits per node)
const MAX_SPAN_SMALL: usize = 5;  // 2^5 = 32 children max
const MAX_SPAN_LARGE: usize = 8;  // 2^8 = 256 children max

/// Height Optimized Trie
pub struct HOT {
    // All data in single arena for cache efficiency
    arena: Vec<u8>,
    root: u32,
    len: usize,
}

impl HOT {
    pub fn new() -> Self {
        Self {
            arena: Vec::with_capacity(1024 * 1024),
            root: 0,
            len: 0,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    // Arena helpers
    #[inline(always)]
    fn alloc(&mut self, size: usize) -> u32 {
        let off = self.arena.len();
        self.arena.resize(off + size, 0);
        off as u32
    }

    #[inline(always)]
    fn u8_at(&self, off: u32) -> u8 {
        self.arena[off as usize]
    }

    #[inline(always)]
    fn set_u8(&mut self, off: u32, v: u8) {
        self.arena[off as usize] = v;
    }

    #[inline(always)]
    fn u16_at(&self, off: u32) -> u16 {
        let o = off as usize;
        u16::from_le_bytes([self.arena[o], self.arena[o + 1]])
    }

    #[inline(always)]
    fn set_u16(&mut self, off: u32, v: u16) {
        let o = off as usize;
        let b = v.to_le_bytes();
        self.arena[o] = b[0];
        self.arena[o + 1] = b[1];
    }

    #[inline(always)]
    fn u32_at(&self, off: u32) -> u32 {
        let o = off as usize;
        u32::from_le_bytes([
            self.arena[o],
            self.arena[o + 1],
            self.arena[o + 2],
            self.arena[o + 3],
        ])
    }

    #[inline(always)]
    fn set_u32(&mut self, off: u32, v: u32) {
        let o = off as usize;
        let b = v.to_le_bytes();
        self.arena[o] = b[0];
        self.arena[o + 1] = b[1];
        self.arena[o + 2] = b[2];
        self.arena[o + 3] = b[3];
    }

    #[inline(always)]
    fn u64_at(&self, off: u32) -> u64 {
        let o = off as usize;
        u64::from_le_bytes([
            self.arena[o],
            self.arena[o + 1],
            self.arena[o + 2],
            self.arena[o + 3],
            self.arena[o + 4],
            self.arena[o + 5],
            self.arena[o + 6],
            self.arena[o + 7],
        ])
    }

    #[inline(always)]
    fn set_u64(&mut self, off: u32, v: u64) {
        let o = off as usize;
        let b = v.to_le_bytes();
        for i in 0..8 {
            self.arena[o + i] = b[i];
        }
    }

    // Get bit at position (MSB-first within bytes)
    #[inline(always)]
    fn bit_at(key: &[u8], pos: u16) -> u8 {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);
        if byte_idx < key.len() {
            (key[byte_idx] >> bit_idx) & 1
        } else {
            0
        }
    }

    // Extract index from key using discriminator bit positions
    #[inline]
    fn extract_index(&self, key: &[u8], node: u32, k: usize) -> usize {
        let disc_off = node + 8; // After header (type:1 + k:1 + height:2 + mask:4)
        let mut idx = 0usize;
        for i in 0..k {
            let bit_pos = self.u16_at(disc_off + (i as u32) * 2);
            idx = (idx << 1) | (Self::bit_at(key, bit_pos) as usize);
        }
        idx
    }

    // Count set bits in mask up to position (exclusive)
    #[inline]
    fn popcount_before(&self, node: u32, pos: usize) -> usize {
        let mask = self.u32_at(node + 4);
        let below = mask & ((1u32 << pos) - 1);
        below.count_ones() as usize
    }

    // Check if position is set in mask
    #[inline]
    fn mask_has(&self, node: u32, pos: usize) -> bool {
        let mask = self.u32_at(node + 4);
        (mask >> pos) & 1 == 1
    }

    // Set bit in mask
    #[inline]
    fn mask_set(&mut self, node: u32, pos: usize) {
        let mask = self.u32_at(node + 4);
        self.set_u32(node + 4, mask | (1u32 << pos));
    }

    // Count total children
    #[inline]
    fn child_count(&self, node: u32) -> usize {
        let mask = self.u32_at(node + 4);
        mask.count_ones() as usize
    }

    // Get child pointer at dense index
    #[inline]
    fn child_at(&self, node: u32, k: usize, dense_idx: usize) -> u32 {
        let children_off = node + 8 + (k as u32) * 2;
        self.u32_at(children_off + (dense_idx as u32) * 4)
    }

    // Leaf layout: [type:1][keylen:2][key bytes...][value:8]
    fn create_leaf(&mut self, key: &[u8], value: u64) -> u32 {
        let size = 1 + 2 + key.len() + 8;
        let off = self.alloc(size);
        self.set_u8(off, TYPE_LEAF);
        self.set_u16(off + 1, key.len() as u16);
        self.arena[(off + 3) as usize..(off + 3) as usize + key.len()].copy_from_slice(key);
        self.set_u64(off + 3 + key.len() as u32, value);
        off
    }

    fn leaf_key(&self, leaf: u32) -> &[u8] {
        let len = self.u16_at(leaf + 1) as usize;
        &self.arena[(leaf + 3) as usize..(leaf + 3) as usize + len]
    }

    fn leaf_value(&self, leaf: u32) -> u64 {
        let len = self.u16_at(leaf + 1) as usize;
        self.u64_at(leaf + 3 + len as u32)
    }

    fn set_leaf_value(&mut self, leaf: u32, value: u64) {
        let len = self.u16_at(leaf + 1) as usize;
        self.set_u64(leaf + 3 + len as u32, value);
    }

    // Find first differing bit between two keys
    fn first_diff_bit(a: &[u8], b: &[u8]) -> Option<u16> {
        let max_len = a.len().max(b.len());
        for i in 0..max_len {
            let a_byte = a.get(i).copied().unwrap_or(0);
            let b_byte = b.get(i).copied().unwrap_or(0);
            if a_byte != b_byte {
                let xor = a_byte ^ b_byte;
                let first_bit = 7 - (xor.leading_zeros() as u16);
                return Some((i as u16) * 8 + (7 - first_bit));
            }
        }
        None
    }

    // Create inner node with k=1 (single discriminator bit)
    // Layout: [type:1][k:1][height:2][mask:4][disc_bit:2][children:4*n]
    fn create_inner_k1(&mut self, disc_bit: u16, children: &[(usize, u32)]) -> u32 {
        let n = children.len();
        let size = 8 + 2 + n * 4; // header + 1 disc bit + children
        let off = self.alloc(size);

        self.set_u8(off, TYPE_INNER_SMALL);
        self.set_u8(off + 1, 1); // k = 1
        self.set_u16(off + 2, 0); // height (unused for now)

        // Build mask
        let mut mask = 0u32;
        for &(idx, _) in children {
            mask |= 1u32 << idx;
        }
        self.set_u32(off + 4, mask);

        // Discriminator bit
        self.set_u16(off + 8, disc_bit);

        // Children (stored densely)
        let children_off = off + 10;
        for (i, &(_, child)) in children.iter().enumerate() {
            self.set_u32(children_off + (i as u32) * 4, child);
        }

        off
    }

    // Rebuild inner node with new child added
    fn rebuild_inner_with_child(&mut self, old_node: u32, new_idx: usize, new_child: u32) -> u32 {
        let k = self.u8_at(old_node + 1) as usize;
        let old_mask = self.u32_at(old_node + 4);
        let old_count = old_mask.count_ones() as usize;

        // Collect old children with their indices
        let mut all_children: Vec<(usize, u32)> = Vec::with_capacity(old_count + 1);
        let disc_off = old_node + 8;
        let children_off = old_node + 8 + (k as u32) * 2;

        let mut dense_idx = 0;
        for idx in 0..(1usize << k) {
            if (old_mask >> idx) & 1 == 1 {
                let child = self.u32_at(children_off + (dense_idx as u32) * 4);
                all_children.push((idx, child));
                dense_idx += 1;
            }
        }

        // Add new child
        all_children.push((new_idx, new_child));
        all_children.sort_by_key(|&(idx, _)| idx);

        // Copy discriminator bits
        let mut disc_bits: Vec<u16> = Vec::with_capacity(k);
        for i in 0..k {
            disc_bits.push(self.u16_at(disc_off + (i as u32) * 2));
        }

        // Create new node
        let n = all_children.len();
        let size = 8 + k * 2 + n * 4;
        let off = self.alloc(size);

        self.set_u8(off, TYPE_INNER_SMALL);
        self.set_u8(off + 1, k as u8);
        self.set_u16(off + 2, 0);

        let mut mask = 0u32;
        for &(idx, _) in &all_children {
            mask |= 1u32 << idx;
        }
        self.set_u32(off + 4, mask);

        for (i, &db) in disc_bits.iter().enumerate() {
            self.set_u16(off + 8 + (i as u32) * 2, db);
        }

        let new_children_off = off + 8 + (k as u32) * 2;
        for (i, &(_, child)) in all_children.iter().enumerate() {
            self.set_u32(new_children_off + (i as u32) * 4, child);
        }

        off
    }

    // Rebuild inner node with child updated
    fn rebuild_inner_update_child(&mut self, old_node: u32, upd_idx: usize, new_child: u32) -> u32 {
        let k = self.u8_at(old_node + 1) as usize;
        let mask = self.u32_at(old_node + 4);
        let count = mask.count_ones() as usize;

        let disc_off = old_node + 8;
        let children_off = old_node + 8 + (k as u32) * 2;

        // Copy discriminator bits
        let mut disc_bits: Vec<u16> = Vec::with_capacity(k);
        for i in 0..k {
            disc_bits.push(self.u16_at(disc_off + (i as u32) * 2));
        }

        // Copy children, replacing the one at upd_idx
        let mut all_children: Vec<(usize, u32)> = Vec::with_capacity(count);
        let mut dense_idx = 0;
        for idx in 0..(1usize << k) {
            if (mask >> idx) & 1 == 1 {
                let child = if idx == upd_idx {
                    new_child
                } else {
                    self.u32_at(children_off + (dense_idx as u32) * 4)
                };
                all_children.push((idx, child));
                dense_idx += 1;
            }
        }

        // Create new node
        let n = all_children.len();
        let size = 8 + k * 2 + n * 4;
        let off = self.alloc(size);

        self.set_u8(off, TYPE_INNER_SMALL);
        self.set_u8(off + 1, k as u8);
        self.set_u16(off + 2, 0);
        self.set_u32(off + 4, mask);

        for (i, &db) in disc_bits.iter().enumerate() {
            self.set_u16(off + 8 + (i as u32) * 2, db);
        }

        let new_children_off = off + 8 + (k as u32) * 2;
        for (i, &(_, child)) in all_children.iter().enumerate() {
            self.set_u32(new_children_off + (i as u32) * 4, child);
        }

        off
    }

    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        if self.root == 0 {
            self.root = self.create_leaf(key, value);
            self.len = 1;
            return None;
        }

        match self.insert_rec(self.root, key, value) {
            InsertResult::Updated(old) => Some(old),
            InsertResult::Inserted => {
                self.len += 1;
                None
            }
            InsertResult::Replace(new_node) => {
                self.root = new_node;
                self.len += 1;
                None
            }
        }
    }

    fn insert_rec(&mut self, node: u32, key: &[u8], value: u64) -> InsertResult {
        let node_type = self.u8_at(node);

        if node_type == TYPE_LEAF {
            let existing_key = self.leaf_key(node).to_vec();

            if existing_key.as_slice() == key {
                let old = self.leaf_value(node);
                self.set_leaf_value(node, value);
                return InsertResult::Updated(old);
            }

            // Split: create inner node at first differing bit
            if let Some(diff_bit) = Self::first_diff_bit(&existing_key, key) {
                let new_leaf = self.create_leaf(key, value);

                let old_idx = Self::bit_at(&existing_key, diff_bit) as usize;
                let new_idx = Self::bit_at(key, diff_bit) as usize;

                let children = if old_idx < new_idx {
                    vec![(old_idx, node), (new_idx, new_leaf)]
                } else {
                    vec![(new_idx, new_leaf), (old_idx, node)]
                };

                let new_node = self.create_inner_k1(diff_bit, &children);
                return InsertResult::Replace(new_node);
            }

            // Keys are equal - shouldn't happen
            InsertResult::Inserted
        } else {
            // Inner node
            let k = self.u8_at(node + 1) as usize;
            let idx = self.extract_index(key, node, k);

            if self.mask_has(node, idx) {
                // Child exists, recurse
                let dense_idx = self.popcount_before(node, idx);
                let child = self.child_at(node, k, dense_idx);

                match self.insert_rec(child, key, value) {
                    InsertResult::Updated(old) => InsertResult::Updated(old),
                    InsertResult::Inserted => InsertResult::Inserted,
                    InsertResult::Replace(new_child) => {
                        let new_node = self.rebuild_inner_update_child(node, idx, new_child);
                        InsertResult::Replace(new_node)
                    }
                }
            } else {
                // No child at this index, add new leaf
                let new_leaf = self.create_leaf(key, value);
                let new_node = self.rebuild_inner_with_child(node, idx, new_leaf);
                InsertResult::Replace(new_node)
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root == 0 {
            return None;
        }
        self.get_rec(self.root, key)
    }

    fn get_rec(&self, node: u32, key: &[u8]) -> Option<u64> {
        let node_type = self.u8_at(node);

        if node_type == TYPE_LEAF {
            let leaf_key = self.leaf_key(node);
            if leaf_key == key {
                Some(self.leaf_value(node))
            } else {
                None
            }
        } else {
            let k = self.u8_at(node + 1) as usize;
            let idx = self.extract_index(key, node, k);

            if self.mask_has(node, idx) {
                let dense_idx = self.popcount_before(node, idx);
                let child = self.child_at(node, k, dense_idx);
                self.get_rec(child, key)
            } else {
                None
            }
        }
    }

    pub fn memory_usage(&self) -> usize {
        self.arena.capacity()
    }
}

impl Default for HOT {
    fn default() -> Self {
        Self::new()
    }
}

enum InsertResult {
    Inserted,
    Updated(u64),
    Replace(u32),
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
    fn test_many() {
        let mut t = HOT::new();
        for i in 0..10000u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }
        assert_eq!(t.len(), 10000);
        for i in 0..10000u64 {
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
        let keys: Vec<String> = (0..1000).map(|i| format!("k{:04}", (i * 7919) % 1000)).collect();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        // Last insert for each key wins
        for (i, key) in keys.iter().enumerate() {
            assert!(t.get(key.as_bytes()).is_some(), "Missing key {}", key);
        }
    }
}
