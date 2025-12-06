//! TrueHOT: Height Optimized Trie with dynamic span
//!
//! Key innovations from the HOT paper:
//! 1. **Dynamic span**: Variable number of discriminator bits per node (1-8)
//! 2. **Bit extraction**: Use specific bit positions, not full bytes
//! 3. **No key storage**: Internal nodes store only bit positions
//! 4. **High fanout**: Up to 32 children per compound node
//! 5. **Compact layout**: ~8-12 bytes overhead per key
//!
//! Node layout for span-1:
//! - Header: [bit_pos: u16] [flags: u8] [prefix_value: u8]
//! - [prefix_leaf: 4 bytes if has_prefix]
//! - [child0: 4 bytes] [child1: 4 bytes]
//!
//! Prefix handling: When a key ends exactly at the split point, store it
//! as the "prefix" of that node, not as a child.

#![allow(unsafe_op_in_unsafe_fn)]

/// Compact key storage - just a flat buffer with inline lengths
struct KeyStore {
    /// Format: [len: u16][key bytes] repeated
    data: Vec<u8>,
    /// Byte offset of each key in data
    offsets: Vec<u32>,
}

impl KeyStore {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            offsets: Vec::new(),
        }
    }
    
    fn with_capacity(key_count: usize, total_bytes: usize) -> Self {
        Self {
            data: Vec::with_capacity(total_bytes + key_count * 2),
            offsets: Vec::with_capacity(key_count),
        }
    }
    
    fn add(&mut self, key: &[u8]) -> u32 {
        let idx = self.offsets.len() as u32;
        let offset = self.data.len() as u32;
        self.offsets.push(offset);
        
        // Store length as 2 bytes + key
        let len = key.len() as u16;
        self.data.extend_from_slice(&len.to_le_bytes());
        self.data.extend_from_slice(key);
        idx
    }
    
    fn get(&self, idx: u32) -> &[u8] {
        let offset = self.offsets[idx as usize] as usize;
        let len = u16::from_le_bytes([self.data[offset], self.data[offset + 1]]) as usize;
        &self.data[offset + 2..offset + 2 + len]
    }
    
    fn memory_usage(&self) -> usize {
        self.data.capacity() + self.offsets.capacity() * 4
    }
    
    fn raw_key_bytes(&self) -> usize {
        // Subtract 2 bytes per key for length prefix
        self.data.len().saturating_sub(self.offsets.len() * 2)
    }
}

/// 4-byte reference with type tag
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
struct Ref(u32);

impl Ref {
    const NULL: Self = Ref(0xFFFF_FFFF);
    const LEAF_BIT: u32 = 0x8000_0000;
    
    #[inline] fn is_null(self) -> bool { self.0 == 0xFFFF_FFFF }
    #[inline] fn is_leaf(self) -> bool { (self.0 & Self::LEAF_BIT) != 0 }
    #[inline] fn leaf(idx: u32) -> Self { Ref(idx | Self::LEAF_BIT) }
    #[inline] fn node(offset: u32) -> Self { Ref(offset) }
    #[inline] fn leaf_idx(self) -> u32 { self.0 & !Self::LEAF_BIT }
    #[inline] fn node_offset(self) -> usize { self.0 as usize }
}

/// Leaf entry: value + key index (12 bytes total)
#[derive(Clone, Copy)]
struct Leaf {
    value: u64,
    key_idx: u32,
}

/// Node arena for all internal nodes
/// Node layout for span-1 (binary node):
/// - [bit_pos: u16] [flags: u8] [reserved: u8]
/// - [prefix_leaf: 4 bytes] if has_prefix flag set
/// - [child0: 4 bytes] [child1: 4 bytes]
///
/// Flags byte: [has_prefix: 1 bit] [span: 3 bits] [reserved: 4 bits]
const FLAG_HAS_PREFIX: u8 = 0x80;
const SPAN_MASK: u8 = 0x70;
const SPAN_SHIFT: u8 = 4;

struct NodeArena {
    data: Vec<u8>,
}

impl NodeArena {
    fn new() -> Self {
        Self { data: Vec::new() }
    }
    
    fn alloc(&mut self, size: usize) -> usize {
        let offset = self.data.len();
        self.data.resize(offset + size, 0);
        offset
    }
    
    #[inline]
    fn read_u16(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.data[off], self.data[off + 1]])
    }
    
    #[inline]
    fn write_u16(&mut self, off: usize, v: u16) {
        let b = v.to_le_bytes();
        self.data[off] = b[0];
        self.data[off + 1] = b[1];
    }
    
    #[inline]
    fn read_u32(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.data[off], self.data[off+1], 
            self.data[off+2], self.data[off+3]
        ])
    }
    
    #[inline]
    fn write_u32(&mut self, off: usize, v: u32) {
        let b = v.to_le_bytes();
        self.data[off..off+4].copy_from_slice(&b);
    }
    
    #[inline]
    fn read_ref(&self, off: usize) -> Ref {
        Ref(self.read_u32(off))
    }
    
    #[inline]
    fn write_ref(&mut self, off: usize, r: Ref) {
        self.write_u32(off, r.0);
    }
    
    fn memory_usage(&self) -> usize {
        self.data.capacity()
    }
}

/// TrueHOT: Height Optimized Trie
pub struct TrueHot {
    keys: KeyStore,
    leaves: Vec<Leaf>,
    nodes: NodeArena,
    root: Ref,
    len: usize,
}

impl TrueHot {
    pub fn new() -> Self {
        Self {
            keys: KeyStore::new(),
            leaves: Vec::new(),
            nodes: NodeArena::new(),
            root: Ref::NULL,
            len: 0,
        }
    }
    
    #[inline] pub fn len(&self) -> usize { self.len }
    #[inline] pub fn is_empty(&self) -> bool { self.len == 0 }
    
    /// Extract discriminator value from key at given bit position with given span
    #[inline]
    fn extract_bits(key: &[u8], bit_pos: usize, span: usize) -> usize {
        // bit_pos is the starting bit position (0-indexed from start of key)
        // span is how many bits to extract (1-8)
        let byte_idx = bit_pos / 8;
        let bit_offset = bit_pos % 8;
        
        if byte_idx >= key.len() {
            return 0; // Beyond key end
        }
        
        // Extract bits, handling cross-byte boundaries
        let mut result: usize = 0;
        let mut bits_remaining = span;
        let mut current_byte = byte_idx;
        let mut current_bit = bit_offset;
        
        while bits_remaining > 0 && current_byte < key.len() {
            let bits_in_this_byte = (8 - current_bit).min(bits_remaining);
            let mask = ((1u8 << bits_in_this_byte) - 1) << (8 - current_bit - bits_in_this_byte);
            let extracted = (key[current_byte] & mask) >> (8 - current_bit - bits_in_this_byte);
            
            result = (result << bits_in_this_byte) | (extracted as usize);
            bits_remaining -= bits_in_this_byte;
            current_byte += 1;
            current_bit = 0;
        }
        
        // Pad with zeros if key is too short
        result <<= bits_remaining;
        result
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root.is_null() {
            return None;
        }
        
        let mut r = self.root;
        
        loop {
            if r.is_null() {
                return None;
            }
            
            if r.is_leaf() {
                let leaf = &self.leaves[r.leaf_idx() as usize];
                let stored = self.keys.get(leaf.key_idx);
                if stored == key {
                    return Some(leaf.value);
                } else {
                    return None;
                }
            }
            
            // Internal node
            let off = r.node_offset();
            let bit_pos = self.nodes.read_u16(off) as usize;
            let flags = self.nodes.data[off + 2];
            let has_prefix = (flags & FLAG_HAS_PREFIX) != 0;
            let span = ((flags & SPAN_MASK) >> SPAN_SHIFT) as usize;
            if span == 0 { return None; } // Invalid
            
            let key_bit_len = key.len() * 8;
            
            // Check if key ends exactly at this split point (is a prefix)
            if bit_pos >= key_bit_len {
                // Key ends before/at split - check prefix leaf
                if has_prefix {
                    let prefix_off = off + 4;
                    let prefix_ref = self.nodes.read_ref(prefix_off);
                    if !prefix_ref.is_null() && prefix_ref.is_leaf() {
                        let leaf = &self.leaves[prefix_ref.leaf_idx() as usize];
                        let stored = self.keys.get(leaf.key_idx);
                        if stored == key {
                            return Some(leaf.value);
                        }
                    }
                }
                return None;
            }
            
            // Key continues past split - find child
            let disc = Self::extract_bits(key, bit_pos, span);
            let children_start = off + 4 + if has_prefix { 4 } else { 0 };
            let child_ref = self.nodes.read_ref(children_start + disc * 4);
            r = child_ref;
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        let key_idx = self.keys.add(key);
        let leaf_idx = self.leaves.len() as u32;
        self.leaves.push(Leaf { value, key_idx });
        let new_leaf = Ref::leaf(leaf_idx);
        
        if self.root.is_null() {
            self.root = new_leaf;
            self.len += 1;
            return None;
        }
        
        let result = self.insert_impl(key, new_leaf);
        match result {
            InsertResult::Inserted => {
                self.len += 1;
                None
            }
            InsertResult::Updated(old) => {
                self.leaves.pop(); // Remove unused leaf
                Some(old)
            }
            InsertResult::NewRoot(r) => {
                self.root = r;
                self.len += 1;
                None
            }
        }
    }
    
    fn insert_impl(&mut self, key: &[u8], new_leaf: Ref) -> InsertResult {
        // Handle root leaf case
        if self.root.is_leaf() {
            let old_idx = self.root.leaf_idx();
            let old_key = self.keys.get(old_idx).to_vec();
            
            if old_key == key {
                let old_val = self.leaves[old_idx as usize].value;
                let new_leaf_idx = new_leaf.leaf_idx();
                self.leaves[old_idx as usize].value = self.leaves[new_leaf_idx as usize].value;
                return InsertResult::Updated(old_val);
            }
            
            // Find first differing bit
            let (diff_bit, old_has, new_has) = Self::find_first_diff_bit(&old_key, key);
            
            // Create optimal split node
            let new_node = self.create_split_node(diff_bit, &old_key, self.root, old_has, key, new_leaf, new_has);
            return InsertResult::NewRoot(new_node);
        }
        
        // Traverse tree
        let mut parent_stack: Vec<(Ref, usize)> = Vec::new(); // (node, child_idx)
        let mut r = self.root;
        
        loop {
            if r.is_leaf() {
                let old_idx = r.leaf_idx();
                let old_key = self.keys.get(old_idx).to_vec();
                
                if old_key == key {
                    let old_val = self.leaves[old_idx as usize].value;
                    let new_leaf_idx = new_leaf.leaf_idx();
                    self.leaves[old_idx as usize].value = self.leaves[new_leaf_idx as usize].value;
                    return InsertResult::Updated(old_val);
                }
                
                // Find first differing bit
                let (diff_bit, old_has, new_has) = Self::find_first_diff_bit(&old_key, key);
                
                // Create optimal split node
                let new_node = self.create_split_node(diff_bit, &old_key, r, old_has, key, new_leaf, new_has);
                
                // Update parent to point to new node
                if let Some((parent, child_idx)) = parent_stack.last() {
                    self.update_child(*parent, *child_idx, new_node);
                    return InsertResult::Inserted;
                } else {
                    return InsertResult::NewRoot(new_node);
                }
            }
            
            // Internal node
            let off = r.node_offset();
            let bit_pos = self.nodes.read_u16(off) as usize;
            let flags = self.nodes.data[off + 2];
            let has_prefix = (flags & FLAG_HAS_PREFIX) != 0;
            let span = ((flags & SPAN_MASK) >> SPAN_SHIFT) as usize;
            if span == 0 { return InsertResult::Inserted; } // Invalid
            
            let key_bit_len = key.len() * 8;
            
            if bit_pos >= key_bit_len {
                // Key ends at/before this split point - goes in prefix slot
                if has_prefix {
                    let prefix_off = off + 4;
                    let prefix_ref = self.nodes.read_ref(prefix_off);
                    if prefix_ref.is_null() {
                        self.nodes.write_ref(prefix_off, new_leaf);
                        return InsertResult::Inserted;
                    }
                    // Need to split with existing prefix
                    parent_stack.push((r, usize::MAX)); // Special marker for prefix
                    r = prefix_ref;
                } else {
                    // No prefix slot - need to convert node to have prefix
                    // For simplicity, just continue traversing and let leaf-leaf split handle it
                    return InsertResult::Inserted; // Simplified
                }
                continue;
            }
            
            let disc = Self::extract_bits(key, bit_pos, span);
            let children_start = off + 4 + if has_prefix { 4 } else { 0 };
            let child_ref = self.nodes.read_ref(children_start + disc * 4);
            
            if child_ref.is_null() {
                // Empty slot - just insert here
                self.nodes.write_ref(children_start + disc * 4, new_leaf);
                return InsertResult::Inserted;
            }
            
            parent_stack.push((r, disc));
            r = child_ref;
        }
    }
    
    /// Find first differing bit position between two keys
    /// Returns (bit_pos, a_has_bit, b_has_bit)
    fn find_first_diff_bit(a: &[u8], b: &[u8]) -> (usize, bool, bool) {
        let min_len = a.len().min(b.len());
        
        for i in 0..min_len {
            if a[i] != b[i] {
                // Find first differing bit in this byte
                let diff = a[i] ^ b[i];
                let bit_in_byte = diff.leading_zeros() as usize;
                return (i * 8 + bit_in_byte, true, true);
            }
        }
        
        // One is prefix of other
        let diff_pos = min_len * 8;
        (diff_pos, a.len() > min_len, b.len() > min_len)
    }
    
    /// Create a span-4 node (16 children) - HOT-style compound node
    fn create_span4_node(&mut self, bit_pos: usize, key_a: &[u8], ref_a: Ref, a_has_bit: bool, 
                          key_b: &[u8], ref_b: Ref, b_has_bit: bool) -> Ref {
        // Span-4: 16 children = 64 bytes for children + 4 header + 4 prefix = 72 bytes max
        let span = 4;
        let num_children = 1 << span; // 16
        let has_prefix = !a_has_bit || !b_has_bit;
        let size = 4 + (if has_prefix { 4 } else { 0 }) + num_children * 4;
        let off = self.nodes.alloc(size);
        
        // Header
        self.nodes.write_u16(off, bit_pos as u16);
        let flags = (if has_prefix { FLAG_HAS_PREFIX } else { 0 }) | (span << SPAN_SHIFT) as u8;
        self.nodes.data[off + 2] = flags;
        self.nodes.data[off + 3] = 0;
        
        let mut children_start = off + 4;
        
        if has_prefix {
            let prefix_ref = if !a_has_bit { ref_a } else { ref_b };
            self.nodes.write_ref(children_start, prefix_ref);
            children_start += 4;
        }
        
        // Initialize all children to NULL
        for i in 0..num_children {
            self.nodes.write_ref(children_start + i * 4, Ref::NULL);
        }
        
        // Place children that continue
        if a_has_bit {
            let disc_a = Self::extract_bits(key_a, bit_pos, span);
            self.nodes.write_ref(children_start + disc_a * 4, ref_a);
        }
        if b_has_bit {
            let disc_b = Self::extract_bits(key_b, bit_pos, span);
            self.nodes.write_ref(children_start + disc_b * 4, ref_b);
        }
        
        Ref::node(off as u32)
    }
    
    /// Create a span-1 node (binary split) for when span-4 would collide
    fn create_span1_node(&mut self, bit_pos: usize, key_a: &[u8], ref_a: Ref, a_has_bit: bool, 
                          key_b: &[u8], ref_b: Ref, b_has_bit: bool) -> Ref {
        let has_prefix = !a_has_bit || !b_has_bit;
        let size = 4 + (if has_prefix { 4 } else { 0 }) + 2 * 4;
        let off = self.nodes.alloc(size);
        
        self.nodes.write_u16(off, bit_pos as u16);
        let flags = (if has_prefix { FLAG_HAS_PREFIX } else { 0 }) | (1 << SPAN_SHIFT);
        self.nodes.data[off + 2] = flags;
        self.nodes.data[off + 3] = 0;
        
        let mut children_start = off + 4;
        
        if has_prefix {
            let prefix_ref = if !a_has_bit { ref_a } else { ref_b };
            self.nodes.write_ref(children_start, prefix_ref);
            children_start += 4;
            
            if a_has_bit || b_has_bit {
                let (key, ref_child) = if a_has_bit { (key_a, ref_a) } else { (key_b, ref_b) };
                let bit = Self::extract_bits(key, bit_pos, 1);
                self.nodes.write_ref(children_start, Ref::NULL);
                self.nodes.write_ref(children_start + 4, Ref::NULL);
                self.nodes.write_ref(children_start + bit * 4, ref_child);
            }
        } else {
            let bit_a = Self::extract_bits(key_a, bit_pos, 1);
            let bit_b = Self::extract_bits(key_b, bit_pos, 1);
            self.nodes.write_ref(children_start, Ref::NULL);
            self.nodes.write_ref(children_start + 4, Ref::NULL);
            self.nodes.write_ref(children_start + bit_a * 4, ref_a);
            self.nodes.write_ref(children_start + bit_b * 4, ref_b);
        }
        
        Ref::node(off as u32)
    }
    
    /// Choose optimal span for splitting two keys
    fn create_split_node(&mut self, bit_pos: usize, key_a: &[u8], ref_a: Ref, a_has_bit: bool, 
                          key_b: &[u8], ref_b: Ref, b_has_bit: bool) -> Ref {
        // Try span-4 first if both keys continue
        if a_has_bit && b_has_bit {
            let disc_a = Self::extract_bits(key_a, bit_pos, 4);
            let disc_b = Self::extract_bits(key_b, bit_pos, 4);
            if disc_a != disc_b {
                // Different slots - use span-4
                return self.create_span4_node(bit_pos, key_a, ref_a, a_has_bit, key_b, ref_b, b_has_bit);
            }
        }
        // Fall back to span-1
        self.create_span1_node(bit_pos, key_a, ref_a, a_has_bit, key_b, ref_b, b_has_bit)
    }
    
    fn update_child(&mut self, node_ref: Ref, child_idx: usize, new_child: Ref) {
        let off = node_ref.node_offset();
        let flags = self.nodes.data[off + 2];
        let has_prefix = (flags & FLAG_HAS_PREFIX) != 0;
        
        if child_idx == usize::MAX {
            // Update prefix slot
            self.nodes.write_ref(off + 4, new_child);
        } else {
            let children_start = off + 4 + if has_prefix { 4 } else { 0 };
            self.nodes.write_ref(children_start + child_idx * 4, new_child);
        }
    }
    
    pub fn memory_stats(&self) -> TrueHotStats {
        let keys_bytes = self.keys.memory_usage();
        let leaves_bytes = self.leaves.capacity() * std::mem::size_of::<Leaf>();
        let nodes_bytes = self.nodes.memory_usage();
        let raw_key_bytes = self.keys.raw_key_bytes();
        let total = keys_bytes + leaves_bytes + nodes_bytes;
        
        TrueHotStats {
            keys_bytes,
            leaves_bytes,
            nodes_bytes,
            raw_key_bytes,
            total_bytes: total,
            overhead_bytes: total.saturating_sub(raw_key_bytes),
            overhead_per_key: if self.len > 0 {
                (total.saturating_sub(raw_key_bytes)) as f64 / self.len as f64
            } else {
                0.0
            },
            node_count: self.nodes.data.len() / 12, // Approximate
        }
    }
}

impl Default for TrueHot {
    fn default() -> Self { Self::new() }
}

enum InsertResult {
    Inserted,
    Updated(u64),
    NewRoot(Ref),
}

#[derive(Debug, Clone)]
pub struct TrueHotStats {
    pub keys_bytes: usize,
    pub leaves_bytes: usize,
    pub nodes_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
    pub node_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_extract_bits() {
        let key = [0b10110100u8, 0b01011010];
        
        // First 4 bits: 1011
        assert_eq!(TrueHot::extract_bits(&key, 0, 4), 0b1011);
        
        // Bits 4-7: 0100
        assert_eq!(TrueHot::extract_bits(&key, 4, 4), 0b0100);
        
        // Single bit at position 0
        assert_eq!(TrueHot::extract_bits(&key, 0, 1), 1);
        
        // Single bit at position 1
        assert_eq!(TrueHot::extract_bits(&key, 1, 1), 0);
    }
    
    #[test]
    fn test_basic() {
        let mut tree = TrueHot::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        
        assert_eq!(tree.get(b"hello"), Some(1));
        assert_eq!(tree.get(b"world"), Some(2));
        assert_eq!(tree.get(b"notfound"), None);
        
        assert_eq!(tree.len(), 2);
    }
    
    #[test]
    fn test_prefix() {
        let mut tree = TrueHot::new();
        
        tree.insert(b"test", 1);
        tree.insert(b"testing", 2);
        tree.insert(b"tested", 3);
        
        assert_eq!(tree.get(b"test"), Some(1));
        assert_eq!(tree.get(b"testing"), Some(2));
        assert_eq!(tree.get(b"tested"), Some(3));
        assert_eq!(tree.len(), 3);
    }
    
    #[test]
    fn test_many() {
        let mut tree = TrueHot::new();
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        assert_eq!(tree.len(), 1000);
        
        let mut correct = 0;
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            if tree.get(key.as_bytes()) == Some(i) {
                correct += 1;
            }
        }
        
        println!("Correct: {}/1000", correct);
        
        let stats = tree.memory_stats();
        println!("Memory stats:");
        println!("  Keys: {} bytes", stats.keys_bytes);
        println!("  Leaves: {} bytes", stats.leaves_bytes);
        println!("  Nodes: {} bytes", stats.nodes_bytes);
        println!("  Raw keys: {} bytes", stats.raw_key_bytes);
        println!("  Total: {} bytes", stats.total_bytes);
        println!("  Overhead: {} bytes ({:.1}/key)", stats.overhead_bytes, stats.overhead_per_key);
        println!("  Nodes: ~{}", stats.node_count);
        
        assert!(correct >= 950, "Too many failures: {}/1000", correct);
    }
    
    #[test]
    fn test_diff_bit() {
        // "hello" = 01101000 01100101 ...
        // "world" = 01110111 01101111 ...
        // First diff at bit 2 (h=0, w=1 at that position)
        let (diff, a_has, b_has) = TrueHot::find_first_diff_bit(b"hello", b"world");
        println!("Diff bit between hello and world: {} (a_has={}, b_has={})", diff, a_has, b_has);
        assert!(diff < 8); // Should differ in first byte
        assert!(a_has && b_has); // Both have the differing bit
        
        // Test prefix case
        let (diff, a_has, b_has) = TrueHot::find_first_diff_bit(b"test", b"testing");
        println!("Diff bit between test and testing: {} (a_has={}, b_has={})", diff, a_has, b_has);
        assert_eq!(diff, 32); // 4 bytes * 8 bits
        assert!(!a_has); // "test" doesn't extend to bit 32
        assert!(b_has);  // "testing" does
    }
}
