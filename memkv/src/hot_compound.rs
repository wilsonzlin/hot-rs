//! HOT with Compound Nodes - targeting 10 bytes/key overhead
//!
//! Key insight: Instead of 1 BiNode per key (10 bytes each),
//! use compound nodes with up to 32 entries (amortized ~0.5 bytes/entry)

const MAX_ENTRIES: usize = 32;

/// Pointer: high bit = leaf flag
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ptr(u32);

impl Ptr {
    const LEAF_BIT: u32 = 0x8000_0000;
    const NULL: Ptr = Ptr(u32::MAX);
    
    #[inline] fn leaf(idx: u32) -> Self { Self(idx | Self::LEAF_BIT) }
    #[inline] fn node(off: u32) -> Self { Self(off) }
    #[inline] fn is_null(self) -> bool { self.0 == u32::MAX }
    #[inline] fn is_leaf(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT != 0) }
    #[inline] fn is_node(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT == 0) }
    #[inline] fn leaf_idx(self) -> u32 { self.0 & !Self::LEAF_BIT }
    #[inline] fn node_off(self) -> u32 { self.0 }
}

/// Compound Node Layout:
/// [n:1][k:1][bits:2*k][partial_keys:n][children:4*n]
/// - n: number of entries (2-32)
/// - k: number of discriminator bits (1-8)
/// - bits: k bit positions (u16 each)
/// - partial_keys: n sparse partial keys (u8 each)
/// - children: n child pointers (u32 each)
/// 
/// Size: 2 + 2k + n + 4n = 2 + 2k + 5n
/// For k=8, n=32: 2 + 16 + 160 = 178 bytes for 32 entries = 5.6 B/entry

/// Leaf: 12 bytes
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct Leaf {
    key_off: u32,
    value: u64,
}

pub struct CompoundHot {
    key_data: Vec<u8>,
    leaves: Vec<Leaf>,
    nodes: Vec<u8>,
    root: Ptr,
}

impl CompoundHot {
    pub fn new() -> Self {
        Self {
            key_data: Vec::new(),
            leaves: Vec::new(),
            nodes: Vec::new(),
            root: Ptr::NULL,
        }
    }

    #[inline] pub fn len(&self) -> usize { self.leaves.len() }
    #[inline] pub fn is_empty(&self) -> bool { self.leaves.is_empty() }

    fn store_key(&mut self, key: &[u8]) -> u32 {
        let off = self.key_data.len() as u32;
        let len = key.len() as u16;
        self.key_data.extend_from_slice(&len.to_le_bytes());
        self.key_data.extend_from_slice(key);
        off
    }

    fn get_key(&self, leaf: &Leaf) -> &[u8] {
        let off = leaf.key_off as usize;
        let len = u16::from_le_bytes([self.key_data[off], self.key_data[off + 1]]) as usize;
        &self.key_data[off + 2..off + 2 + len]
    }

    // Node helpers
    fn r8(&self, o: u32) -> u8 { self.nodes[o as usize] }
    fn w8(&mut self, o: u32, v: u8) { self.nodes[o as usize] = v; }
    fn r16(&self, o: u32) -> u16 {
        u16::from_le_bytes([self.nodes[o as usize], self.nodes[o as usize + 1]])
    }
    fn w16(&mut self, o: u32, v: u16) {
        let b = v.to_le_bytes();
        self.nodes[o as usize] = b[0];
        self.nodes[o as usize + 1] = b[1];
    }
    fn r32(&self, o: u32) -> u32 {
        let i = o as usize;
        u32::from_le_bytes([self.nodes[i], self.nodes[i+1], self.nodes[i+2], self.nodes[i+3]])
    }
    fn w32(&mut self, o: u32, v: u32) {
        let i = o as usize;
        self.nodes[i..i+4].copy_from_slice(&v.to_le_bytes());
    }

    fn node_size(n: usize, k: usize) -> usize {
        2 + 2 * k + n + 4 * n
    }

    fn alloc_node(&mut self, n: usize, k: usize) -> u32 {
        let off = self.nodes.len() as u32;
        self.nodes.resize(self.nodes.len() + Self::node_size(n, k), 0);
        off
    }

    fn node_n(&self, off: u32) -> usize { self.r8(off) as usize }
    fn node_k(&self, off: u32) -> usize { self.r8(off + 1) as usize }
    fn node_bits_off(&self, off: u32) -> u32 { off + 2 }
    fn node_pk_off(&self, off: u32, k: usize) -> u32 { off + 2 + 2 * k as u32 }
    fn node_ch_off(&self, off: u32, k: usize, n: usize) -> u32 { off + 2 + 2 * k as u32 + n as u32 }

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
        let key_off = self.store_key(key);
        let idx = self.leaves.len() as u32;
        self.leaves.push(Leaf { key_off, value });
        Ptr::leaf(idx)
    }

    /// Create a 2-entry node (BiNode equivalent)
    fn create_binode(&mut self, bit: u16, left: Ptr, right: Ptr) -> Ptr {
        let off = self.alloc_node(2, 1);
        self.w8(off, 2);      // n = 2
        self.w8(off + 1, 1);  // k = 1
        self.w16(off + 2, bit);
        self.w8(off + 4, 0);  // left sparse pk
        self.w8(off + 5, 1);  // right sparse pk
        self.w32(off + 6, left.0);
        self.w32(off + 10, right.0);
        Ptr::node(off)
    }

    /// Extract partial key from full key using discriminator bits
    fn extract_pk(&self, key: &[u8], bits_off: u32, k: usize) -> u8 {
        let mut pk = 0u8;
        for i in 0..k {
            let bit_pos = self.r16(bits_off + 2 * i as u32);
            pk |= Self::bit_at(key, bit_pos) << i;
        }
        pk
    }

    /// Search node for matching entry (returns index and child)
    fn search_node(&self, off: u32, key: &[u8]) -> (usize, Ptr) {
        let n = self.node_n(off);
        let k = self.node_k(off);
        let bits_off = self.node_bits_off(off);
        let pk_off = self.node_pk_off(off, k);
        let ch_off = self.node_ch_off(off, k, n);

        let search_pk = self.extract_pk(key, bits_off, k);

        // Find entry whose sparse pk is subset of search pk
        let mut match_idx = 0;
        for i in 0..n {
            let sparse_pk = self.r8(pk_off + i as u32);
            if (search_pk & sparse_pk) == sparse_pk {
                match_idx = i;
            }
        }

        (match_idx, Ptr(self.r32(ch_off + 4 * match_idx as u32)))
    }

    /// Add entry to existing node (returns new node if node was rebuilt)
    fn add_entry_to_node(&mut self, old_off: u32, new_bit: u16, left: Ptr, right: Ptr, 
                          entry_idx: usize, new_bit_value: u8) -> Ptr {
        let n = self.node_n(old_off);
        let k = self.node_k(old_off);
        
        if n >= MAX_ENTRIES {
            // Node is full - create BiNode and return
            return self.create_binode(new_bit, left, right);
        }

        // Check if we need to add a new discriminator bit
        let bits_off = self.node_bits_off(old_off);
        let mut need_new_bit = true;
        let mut bit_idx = 0;
        
        for i in 0..k {
            if self.r16(bits_off + 2 * i as u32) == new_bit {
                need_new_bit = false;
                bit_idx = i;
                break;
            }
        }

        let new_k = if need_new_bit { k + 1 } else { k };
        if new_k > 8 {
            // Too many bits - create BiNode
            return self.create_binode(new_bit, left, right);
        }

        let new_n = n + 1;
        let new_off = self.alloc_node(new_n, new_k);

        self.w8(new_off, new_n as u8);
        self.w8(new_off + 1, new_k as u8);

        // Copy/update discriminator bits
        let new_bits_off = self.node_bits_off(new_off);
        for i in 0..k {
            self.w16(new_bits_off + 2 * i as u32, self.r16(bits_off + 2 * i as u32));
        }
        if need_new_bit {
            self.w16(new_bits_off + 2 * k as u32, new_bit);
            bit_idx = k;
        }

        // Copy partial keys and children, inserting new entry
        let old_pk_off = self.node_pk_off(old_off, k);
        let old_ch_off = self.node_ch_off(old_off, k, n);
        let new_pk_off = self.node_pk_off(new_off, new_k);
        let new_ch_off = self.node_ch_off(new_off, new_k, new_n);

        let new_pk_bit = if need_new_bit { 1u8 << bit_idx } else { 0 };
        
        // Determine insert position (after entry_idx if new_bit_value is 1)
        let insert_pos = if new_bit_value == 1 { entry_idx + 1 } else { entry_idx };

        for i in 0..new_n {
            if i == insert_pos {
                // Insert new entry
                let pk = if new_bit_value == 1 { new_pk_bit } else { 0 };
                self.w8(new_pk_off + i as u32, pk);
                self.w32(new_ch_off + 4 * i as u32, if new_bit_value == 1 { right.0 } else { left.0 });
            } else {
                let old_i = if i < insert_pos { i } else { i - 1 };
                let mut pk = self.r8(old_pk_off + old_i as u32);
                if need_new_bit {
                    // Extend partial key with new bit
                    pk = (pk & ((1 << bit_idx) - 1)) | ((pk >> bit_idx) << (bit_idx + 1));
                    if old_i >= entry_idx && new_bit_value == 0 {
                        pk |= new_pk_bit;
                    }
                }
                self.w8(new_pk_off + i as u32, pk);
                
                // Copy child, but replace the split entry
                let child = if old_i == entry_idx {
                    if new_bit_value == 1 { left } else { right }
                } else {
                    Ptr(self.r32(old_ch_off + 4 * old_i as u32))
                };
                self.w32(new_ch_off + 4 * i as u32, child.0);
            }
        }

        Ptr::node(new_off)
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
            let (entry_idx, child) = self.search_node(off, key);

            match self.insert_into(child, key, value) {
                InsertResult::Updated(old) => InsertResult::Updated(old),
                InsertResult::Replaced(new_child) => {
                    // Need to update child pointer or grow node
                    // For simplicity, just update in place if it's a BiNode replacement
                    let n = self.node_n(off);
                    let k = self.node_k(off);
                    let ch_off = self.node_ch_off(off, k, n);
                    self.w32(ch_off + 4 * entry_idx as u32, new_child.0);
                    InsertResult::NoChange
                }
                InsertResult::NoChange => InsertResult::NoChange,
            }
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root.is_null() { return None; }
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
            let (_, child) = self.search_node(ptr.node_off(), key);
            self.get_from(child, key)
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
    
    pub fn shrink_to_fit(&mut self) {
        self.key_data.shrink_to_fit();
        self.leaves.shrink_to_fit();
        self.nodes.shrink_to_fit();
    }
}

impl Default for CompoundHot {
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
        let mut t = CompoundHot::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        assert_eq!(t.get(b"hello"), Some(1));
        assert_eq!(t.get(b"world"), Some(2));
        assert_eq!(t.get(b"missing"), None);
    }

    #[test]
    fn test_update() {
        let mut t = CompoundHot::new();
        assert_eq!(t.insert(b"key", 1), None);
        assert_eq!(t.insert(b"key", 2), Some(1));
        assert_eq!(t.get(b"key"), Some(2));
    }

    #[test]
    fn test_many() {
        let mut t = CompoundHot::new();
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
}
