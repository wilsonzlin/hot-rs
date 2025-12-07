//! HOT: Height Optimized Trie
//! 
//! From "HOT: A Height Optimized Trie Index for Main-Memory Database Systems"
//! Binna et al., SIGMOD 2018
//!
//! Key concepts:
//! 1. **Compound nodes**: Multiple trie levels compressed into single nodes
//! 2. **Variable span**: 1-8 discriminator bits per node, chosen based on data
//! 3. **Partial keys**: Only store bits that discriminate between children
//! 4. **Dense/Sparse representation**: Bitmap for sparse, array for dense
//!
//! Target: 11-14 bytes/key overhead (not counting values)

use std::ptr;

/// Maximum span (discriminator bits per node)
const MAX_SPAN: usize = 8;

/// Node types
const NODE_LEAF: u8 = 0;
const NODE_SPARSE: u8 = 1;  // Bitmap + dense child array
const NODE_DENSE: u8 = 2;   // Full 2^span children

/// Sparse node layout (for span k with n children):
/// [type:1][span:1][height:2][bitmap:2^k bits / 8 bytes][n × child:4]
/// 
/// Dense node layout (for span k):
/// [type:1][span:1][height:2][2^k × child:4]
///
/// Leaf layout:
/// [type:1][key_len:2][key...][value:8]

pub struct HOT {
    arena: Vec<u8>,
    root: u32,  // 0 = empty
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
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    #[inline]
    fn alloc(&mut self, size: usize) -> u32 {
        let off = self.arena.len() as u32;
        self.arena.resize(self.arena.len() + size, 0);
        off
    }
    
    #[inline]
    fn read_u16(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.arena[off], self.arena[off + 1]])
    }
    
    #[inline]
    fn read_u32(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.arena[off], self.arena[off + 1],
            self.arena[off + 2], self.arena[off + 3]
        ])
    }
    
    #[inline]
    fn read_u64(&self, off: usize) -> u64 {
        u64::from_le_bytes([
            self.arena[off], self.arena[off + 1], self.arena[off + 2], self.arena[off + 3],
            self.arena[off + 4], self.arena[off + 5], self.arena[off + 6], self.arena[off + 7],
        ])
    }
    
    #[inline]
    fn write_u16(&mut self, off: usize, v: u16) {
        self.arena[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    
    #[inline]
    fn write_u32(&mut self, off: usize, v: u32) {
        self.arena[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    
    #[inline]
    fn write_u64(&mut self, off: usize, v: u64) {
        self.arena[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
    
    /// Get bit at position from key (MSB first within bytes)
    #[inline]
    fn get_bit(key: &[u8], bit_pos: usize) -> usize {
        let byte_idx = bit_pos / 8;
        let bit_idx = 7 - (bit_pos % 8);
        if byte_idx >= key.len() {
            0
        } else {
            ((key[byte_idx] >> bit_idx) & 1) as usize
        }
    }
    
    /// Get span bits starting at bit_pos
    #[inline]
    fn get_span_bits(key: &[u8], bit_pos: usize, span: usize) -> usize {
        let mut result = 0;
        for i in 0..span {
            result = (result << 1) | Self::get_bit(key, bit_pos + i);
        }
        result
    }
    
    /// Find first differing bit position between two keys
    fn first_diff_bit(a: &[u8], b: &[u8]) -> Option<usize> {
        let max_len = a.len().max(b.len());
        for i in 0..max_len {
            let a_byte = if i < a.len() { a[i] } else { 0 };
            let b_byte = if i < b.len() { b[i] } else { 0 };
            if a_byte != b_byte {
                let xor = a_byte ^ b_byte;
                let bit_in_byte = 7 - xor.leading_zeros() as usize;
                return Some(i * 8 + (7 - bit_in_byte));
            }
        }
        if a.len() != b.len() {
            Some(a.len().min(b.len()) * 8)
        } else {
            None
        }
    }
    
    /// Create a leaf node
    fn create_leaf(&mut self, key: &[u8], value: u64) -> u32 {
        let size = 1 + 2 + key.len() + 8;
        let off = self.alloc(size) as usize;
        self.arena[off] = NODE_LEAF;
        self.write_u16(off + 1, key.len() as u16);
        self.arena[off + 3..off + 3 + key.len()].copy_from_slice(key);
        self.write_u64(off + 3 + key.len(), value);
        off as u32
    }
    
    /// Get key from leaf
    fn leaf_key(&self, leaf: u32) -> &[u8] {
        let off = leaf as usize;
        let len = self.read_u16(off + 1) as usize;
        &self.arena[off + 3..off + 3 + len]
    }
    
    /// Get value from leaf
    fn leaf_value(&self, leaf: u32) -> u64 {
        let off = leaf as usize;
        let len = self.read_u16(off + 1) as usize;
        self.read_u64(off + 3 + len)
    }
    
    /// Set value in leaf
    fn set_leaf_value(&mut self, leaf: u32, value: u64) {
        let off = leaf as usize;
        let len = self.read_u16(off + 1) as usize;
        self.write_u64(off + 3 + len, value);
    }
    
    /// Create a sparse node with given children
    /// bit_pos: starting bit position for discrimination
    /// span: number of bits to use
    /// children: (index, child_ptr) pairs
    fn create_sparse_node(&mut self, bit_pos: u16, span: u8, children: &[(usize, u32)]) -> u32 {
        let n = children.len();
        let bitmap_bytes = (1usize << span).div_ceil(8);
        let size = 1 + 1 + 2 + bitmap_bytes + n * 4;
        let off = self.alloc(size) as usize;
        
        self.arena[off] = NODE_SPARSE;
        self.arena[off + 1] = span;
        self.write_u16(off + 2, bit_pos);
        
        // Clear bitmap
        for i in 0..bitmap_bytes {
            self.arena[off + 4 + i] = 0;
        }
        
        // Set bitmap bits and write children
        let bitmap_off = off + 4;
        let children_off = off + 4 + bitmap_bytes;
        
        for (i, &(idx, child)) in children.iter().enumerate() {
            // Set bit in bitmap
            let byte_idx = idx / 8;
            let bit_idx = idx % 8;
            self.arena[bitmap_off + byte_idx] |= 1 << bit_idx;
            // Write child
            self.write_u32(children_off + i * 4, child);
        }
        
        off as u32
    }
    
    /// Lookup child in sparse node
    fn sparse_lookup(&self, node: u32, child_idx: usize) -> Option<u32> {
        let off = node as usize;
        let span = self.arena[off + 1] as usize;
        let bitmap_bytes = (1usize << span).div_ceil(8);
        let bitmap_off = off + 4;
        
        // Check if bit is set
        let byte_idx = child_idx / 8;
        let bit_idx = child_idx % 8;
        if (self.arena[bitmap_off + byte_idx] >> bit_idx) & 1 == 0 {
            return None;
        }
        
        // Count bits before this position to find child index
        let mut count = 0;
        for i in 0..child_idx {
            let bi = i / 8;
            let bit = i % 8;
            if (self.arena[bitmap_off + bi] >> bit) & 1 != 0 {
                count += 1;
            }
        }
        
        let children_off = off + 4 + bitmap_bytes;
        Some(self.read_u32(children_off + count * 4))
    }
    
    /// Get span and bit_pos from node
    fn node_info(&self, node: u32) -> (u8, u16) {
        let off = node as usize;
        (self.arena[off + 1], self.read_u16(off + 2))
    }
    
    /// Count children in sparse node
    fn sparse_child_count(&self, node: u32) -> usize {
        let off = node as usize;
        let span = self.arena[off + 1] as usize;
        let bitmap_bytes = (1usize << span).div_ceil(8);
        let bitmap_off = off + 4;
        
        let mut count = 0;
        for i in 0..bitmap_bytes {
            count += self.arena[bitmap_off + i].count_ones() as usize;
        }
        count
    }
    
    /// Get all children from sparse node as (index, ptr) pairs
    fn sparse_children(&self, node: u32) -> Vec<(usize, u32)> {
        let off = node as usize;
        let span = self.arena[off + 1] as usize;
        let bitmap_bytes = (1usize << span).div_ceil(8);
        let bitmap_off = off + 4;
        let children_off = off + 4 + bitmap_bytes;
        
        let mut result = Vec::new();
        let mut child_idx = 0;
        
        for i in 0..(1 << span) {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if (self.arena[bitmap_off + byte_idx] >> bit_idx) & 1 != 0 {
                let ptr = self.read_u32(children_off + child_idx * 4);
                result.push((i, ptr));
                child_idx += 1;
            }
        }
        result
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        if self.root == 0 {
            self.root = self.create_leaf(key, value);
            self.len = 1;
            return None;
        }
        
        let result = self.insert_rec(self.root, key, value, 0);
        match result {
            InsertResult::Updated(old) => Some(old),
            InsertResult::Done => {
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
    
    fn insert_rec(&mut self, node: u32, key: &[u8], value: u64, depth_bits: usize) -> InsertResult {
        let off = node as usize;
        let node_type = self.arena[off];
        
        if node_type == NODE_LEAF {
            let existing_key = self.leaf_key(node).to_vec();
            
            if existing_key == key {
                let old = self.leaf_value(node);
                self.set_leaf_value(node, value);
                return InsertResult::Updated(old);
            }
            
            // Need to split: find first differing bit
            if let Some(diff_bit) = Self::first_diff_bit(&existing_key, key) {
                let new_leaf = self.create_leaf(key, value);
                
                // Create node at the differing bit with span 1
                let bit_pos = diff_bit as u16;
                let old_idx = Self::get_bit(&existing_key, diff_bit);
                let new_idx = Self::get_bit(key, diff_bit);
                
                let children = if old_idx < new_idx {
                    vec![(old_idx, node), (new_idx, new_leaf)]
                } else {
                    vec![(new_idx, new_leaf), (old_idx, node)]
                };
                
                let new_node = self.create_sparse_node(bit_pos, 1, &children);
                return InsertResult::Replace(new_node);
            } else {
                // Keys are equal (shouldn't happen, already checked)
                return InsertResult::Done;
            }
        }
        
        // Internal node
        let (span, bit_pos) = self.node_info(node);
        let child_idx = Self::get_span_bits(key, bit_pos as usize, span as usize);
        
        if let Some(child) = self.sparse_lookup(node, child_idx) {
            let result = self.insert_rec(child, key, value, bit_pos as usize + span as usize);
            match result {
                InsertResult::Replace(new_child) => {
                    // Update child pointer - need to rebuild node
                    let mut children = self.sparse_children(node);
                    for (idx, ptr) in &mut children {
                        if *idx == child_idx {
                            *ptr = new_child;
                            break;
                        }
                    }
                    let new_node = self.create_sparse_node(bit_pos, span, &children);
                    InsertResult::Replace(new_node)
                }
                other => other,
            }
        } else {
            // No child at this index - add new leaf
            let new_leaf = self.create_leaf(key, value);
            let mut children = self.sparse_children(node);
            children.push((child_idx, new_leaf));
            children.sort_by_key(|(idx, _)| *idx);
            let new_node = self.create_sparse_node(bit_pos, span, &children);
            InsertResult::Replace(new_node)
        }
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root == 0 { return None; }
        self.get_rec(self.root, key)
    }
    
    fn get_rec(&self, node: u32, key: &[u8]) -> Option<u64> {
        let off = node as usize;
        let node_type = self.arena[off];
        
        if node_type == NODE_LEAF {
            let leaf_key = self.leaf_key(node);
            if leaf_key == key {
                return Some(self.leaf_value(node));
            }
            return None;
        }
        
        let (span, bit_pos) = self.node_info(node);
        let child_idx = Self::get_span_bits(key, bit_pos as usize, span as usize);
        
        if let Some(child) = self.sparse_lookup(node, child_idx) {
            self.get_rec(child, key)
        } else {
            None
        }
    }
    
    pub fn memory_usage(&self) -> usize {
        self.arena.capacity()
    }
}

impl Default for HOT {
    fn default() -> Self { Self::new() }
}

enum InsertResult {
    Done,
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
        for i in 0..10000 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }
        assert_eq!(t.len(), 10000);
        for i in 0..10000 {
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
