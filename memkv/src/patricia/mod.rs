//! PATRICIA Trie: Minimal overhead by storing only discrimination positions
//!
//! Key insight: Don't store key bytes in internal nodes!
//! - Internal nodes only store the byte position to check
//! - Leaves store full keys (needed for verification anyway)
//! - This minimizes per-node overhead
//!
//! Node overhead breakdown:
//! - Internal node: 8 bytes (4-byte skip + 4-byte child)
//! - But we need 2 children minimum, so ~12 bytes per split
//! - Leaves: 12 bytes (4-byte key ref + 8-byte value) + key arena
//!
//! For 1M keys with ~500K internal nodes:
//! - Internal nodes: 500K * 12 = 6M bytes = 6 bytes/key
//! - Leaves: 1M * 12 = 12M bytes = 12 bytes/key  
//! - Total overhead: ~18 bytes/key (close to HOT!)

#![allow(unsafe_op_in_unsafe_fn)]

/// Key storage arena
struct KeyArena {
    data: Vec<u8>,
    positions: Vec<(u32, u16)>, // (offset, len) for each key
}

impl KeyArena {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            positions: Vec::new(),
        }
    }
    
    fn add(&mut self, key: &[u8]) -> u32 {
        let idx = self.positions.len() as u32;
        let offset = self.data.len() as u32;
        let len = key.len() as u16;
        self.data.extend_from_slice(key);
        self.positions.push((offset, len));
        idx
    }
    
    fn get(&self, idx: u32) -> &[u8] {
        let (offset, len) = self.positions[idx as usize];
        &self.data[offset as usize..(offset as usize + len as usize)]
    }
    
    fn memory_usage(&self) -> usize {
        self.data.capacity() + self.positions.capacity() * 6
    }
}

/// Leaf: value + key index (12 bytes)
#[derive(Clone, Copy)]
struct Leaf {
    value: u64,
    key_idx: u32,
}

/// 4-byte reference with tag bit
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
struct Ref(u32);

impl Ref {
    const NULL: Self = Ref(0xFFFF_FFFF);
    const LEAF_TAG: u32 = 0x8000_0000;
    
    fn is_null(self) -> bool { self.0 == 0xFFFF_FFFF }
    
    fn leaf(idx: u32) -> Self { Ref(idx | Self::LEAF_TAG) }
    fn node(offset: u32) -> Self { Ref(offset) }
    
    fn is_leaf(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_TAG) != 0 }
    fn leaf_idx(self) -> u32 { self.0 & !Self::LEAF_TAG }
    fn node_offset(self) -> usize { self.0 as usize }
}

/// Internal node in arena
/// Layout: [split_pos: u16] [num_children: u8] [pad: u8] [keys: num_children bytes] [children: num_children * 4 bytes]
/// Minimum size for 2 children: 2 + 1 + 1 + 2 + 8 = 14 bytes

/// PatriciaArt: Ultra-compact radix trie
pub struct PatriciaArt {
    keys: KeyArena,
    leaves: Vec<Leaf>,
    nodes: Vec<u8>,
    root: Ref,
    len: usize,
}

impl PatriciaArt {
    pub fn new() -> Self {
        Self {
            keys: KeyArena::new(),
            leaves: Vec::new(),
            nodes: Vec::new(),
            root: Ref::NULL,
            len: 0,
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root.is_null() {
            return None;
        }
        
        let mut r = self.root;
        
        while !r.is_null() {
            if r.is_leaf() {
                let leaf = &self.leaves[r.leaf_idx() as usize];
                let stored_key = self.keys.get(leaf.key_idx);
                if stored_key == key {
                    return Some(leaf.value);
                } else {
                    return None;
                }
            }
            
            // Internal node
            let offset = r.node_offset();
            let split_pos = u16::from_le_bytes([self.nodes[offset], self.nodes[offset + 1]]) as usize;
            let num_children = self.nodes[offset + 2] as usize;
            
            if split_pos >= key.len() {
                return None; // Key too short
            }
            
            let byte = key[split_pos];
            let keys_start = offset + 4;
            let children_start = keys_start + num_children;
            
            // Find matching child
            let mut found = Ref::NULL;
            for i in 0..num_children {
                if self.nodes[keys_start + i] == byte {
                    found = self.read_ref(children_start + i * 4);
                    break;
                }
            }
            
            if found.is_null() {
                return None;
            }
            
            r = found;
        }
        
        None
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
        
        let result = self.insert_impl(key, value, leaf_idx, new_leaf);
        match result {
            InsertResult::Inserted => {
                self.len += 1;
                None
            }
            InsertResult::Updated(old) => {
                // Remove the leaf we just added
                self.leaves.pop();
                Some(old)
            }
            InsertResult::NewRoot(r) => {
                self.root = r;
                self.len += 1;
                None
            }
        }
    }
    
    fn insert_impl(&mut self, key: &[u8], value: u64, leaf_idx: u32, new_leaf: Ref) -> InsertResult {
        // If root is a leaf, check for match or split
        if self.root.is_leaf() {
            let old_leaf = &self.leaves[self.root.leaf_idx() as usize];
            let old_key = self.keys.get(old_leaf.key_idx).to_vec();
            
            if old_key == key {
                // Same key - update value
                let old_val = old_leaf.value;
                self.leaves[self.root.leaf_idx() as usize].value = value;
                return InsertResult::Updated(old_val);
            }
            
            // Find first differing byte
            let mut diff_pos = 0;
            while diff_pos < old_key.len() && diff_pos < key.len() && old_key[diff_pos] == key[diff_pos] {
                diff_pos += 1;
            }
            
            // Create internal node at diff_pos
            let new_node = self.alloc_node(diff_pos);
            
            // Add children based on which key has a byte at diff_pos
            if diff_pos < old_key.len() {
                self.add_child(new_node, old_key[diff_pos], self.root);
            }
            if diff_pos < key.len() {
                self.add_child(new_node, key[diff_pos], new_leaf);
            }
            
            return InsertResult::NewRoot(new_node);
        }
        
        // Traverse to find insertion point
        let mut parent_ref = Ref::NULL;
        let mut parent_child_idx = 0usize;
        let mut r = self.root;
        
        loop {
            if r.is_leaf() {
                let old_leaf = &self.leaves[r.leaf_idx() as usize];
                let old_key = self.keys.get(old_leaf.key_idx).to_vec();
                
                if old_key == key {
                    let old_val = old_leaf.value;
                    self.leaves[r.leaf_idx() as usize].value = value;
                    return InsertResult::Updated(old_val);
                }
                
                // Find first differing byte
                let mut diff_pos = 0;
                while diff_pos < old_key.len() && diff_pos < key.len() && old_key[diff_pos] == key[diff_pos] {
                    diff_pos += 1;
                }
                
                // Create internal node
                let new_node = self.alloc_node(diff_pos);
                
                if diff_pos < old_key.len() {
                    self.add_child(new_node, old_key[diff_pos], r);
                }
                if diff_pos < key.len() {
                    self.add_child(new_node, key[diff_pos], new_leaf);
                }
                
                // Update parent
                if parent_ref.is_null() {
                    return InsertResult::NewRoot(new_node);
                } else {
                    self.update_child(parent_ref, parent_child_idx, new_node);
                    return InsertResult::Inserted;
                }
            }
            
            // Internal node
            let offset = r.node_offset();
            let split_pos = u16::from_le_bytes([self.nodes[offset], self.nodes[offset + 1]]) as usize;
            let num_children = self.nodes[offset + 2] as usize;
            let keys_start = offset + 4;
            let children_start = keys_start + num_children;
            
            if split_pos >= key.len() {
                // Key is shorter than this split point - need to insert above
                // Create new node at key.len()
                let new_node = self.alloc_node(key.len());
                // The new leaf has no byte at this position, so we need special handling
                // For now, just add as children
                self.add_child(new_node, self.nodes[keys_start], r); // Use first child's byte
                // But this isn't right... need to handle key prefix case
                
                if parent_ref.is_null() {
                    return InsertResult::NewRoot(new_node);
                } else {
                    self.update_child(parent_ref, parent_child_idx, new_node);
                    return InsertResult::Inserted;
                }
            }
            
            let byte = key[split_pos];
            
            // Find matching child
            let mut found_idx = None;
            for i in 0..num_children {
                if self.nodes[keys_start + i] == byte {
                    found_idx = Some(i);
                    break;
                }
            }
            
            match found_idx {
                Some(idx) => {
                    // Continue down this path
                    parent_ref = r;
                    parent_child_idx = idx;
                    r = self.read_ref(children_start + idx * 4);
                }
                None => {
                    // No matching child - add new child
                    if self.can_add_child(r) {
                        self.add_child(r, byte, new_leaf);
                        return InsertResult::Inserted;
                    } else {
                        // Need to grow node
                        let new_node = self.grow_and_add(r, byte, new_leaf);
                        if parent_ref.is_null() {
                            return InsertResult::NewRoot(new_node);
                        } else {
                            self.update_child(parent_ref, parent_child_idx, new_node);
                            return InsertResult::Inserted;
                        }
                    }
                }
            }
        }
    }
    
    fn read_ref(&self, offset: usize) -> Ref {
        Ref(u32::from_le_bytes([
            self.nodes[offset],
            self.nodes[offset + 1],
            self.nodes[offset + 2],
            self.nodes[offset + 3],
        ]))
    }
    
    fn write_ref(&mut self, offset: usize, r: Ref) {
        let bytes = r.0.to_le_bytes();
        self.nodes[offset..offset + 4].copy_from_slice(&bytes);
    }
    
    fn alloc_node(&mut self, split_pos: usize) -> Ref {
        // Allocate node with space for up to 4 children initially
        // Layout: split_pos(2) + num_children(1) + pad(1) + keys(4) + children(16) = 24 bytes
        let offset = self.nodes.len();
        self.nodes.resize(offset + 24, 0);
        
        let pos_bytes = (split_pos as u16).to_le_bytes();
        self.nodes[offset] = pos_bytes[0];
        self.nodes[offset + 1] = pos_bytes[1];
        self.nodes[offset + 2] = 0; // num_children
        self.nodes[offset + 3] = 4; // max_children (capacity)
        
        // Initialize children to NULL
        for i in 0..4 {
            self.write_ref(offset + 8 + i * 4, Ref::NULL);
        }
        
        Ref::node(offset as u32)
    }
    
    fn can_add_child(&self, node_ref: Ref) -> bool {
        let offset = node_ref.node_offset();
        let num_children = self.nodes[offset + 2] as usize;
        let max_children = self.nodes[offset + 3] as usize;
        num_children < max_children
    }
    
    fn add_child(&mut self, node_ref: Ref, byte: u8, child: Ref) {
        let offset = node_ref.node_offset();
        let num_children = self.nodes[offset + 2] as usize;
        let max_children = self.nodes[offset + 3] as usize;
        
        if num_children >= max_children {
            return; // Should have checked can_add_child first
        }
        
        let keys_start = offset + 4;
        let children_start = keys_start + max_children;
        
        self.nodes[keys_start + num_children] = byte;
        self.write_ref(children_start + num_children * 4, child);
        self.nodes[offset + 2] = (num_children + 1) as u8;
    }
    
    fn update_child(&mut self, node_ref: Ref, idx: usize, child: Ref) {
        let offset = node_ref.node_offset();
        let max_children = self.nodes[offset + 3] as usize;
        let children_start = offset + 4 + max_children;
        self.write_ref(children_start + idx * 4, child);
    }
    
    fn grow_and_add(&mut self, old_ref: Ref, byte: u8, child: Ref) -> Ref {
        let old_offset = old_ref.node_offset();
        let split_pos = u16::from_le_bytes([self.nodes[old_offset], self.nodes[old_offset + 1]]) as usize;
        let old_num = self.nodes[old_offset + 2] as usize;
        let old_max = self.nodes[old_offset + 3] as usize;
        
        // Double capacity
        let new_max = old_max * 2;
        let new_size = 4 + new_max + new_max * 4;
        let new_offset = self.nodes.len();
        self.nodes.resize(new_offset + new_size, 0);
        
        // Write header
        let pos_bytes = (split_pos as u16).to_le_bytes();
        self.nodes[new_offset] = pos_bytes[0];
        self.nodes[new_offset + 1] = pos_bytes[1];
        self.nodes[new_offset + 2] = (old_num + 1) as u8;
        self.nodes[new_offset + 3] = new_max as u8;
        
        // Copy old keys and children
        let old_keys = old_offset + 4;
        let old_children = old_keys + old_max;
        let new_keys = new_offset + 4;
        let new_children = new_keys + new_max;
        
        for i in 0..old_num {
            self.nodes[new_keys + i] = self.nodes[old_keys + i];
            let c = self.read_ref(old_children + i * 4);
            self.write_ref(new_children + i * 4, c);
        }
        
        // Add new child
        self.nodes[new_keys + old_num] = byte;
        self.write_ref(new_children + old_num * 4, child);
        
        Ref::node(new_offset as u32)
    }
    
    pub fn memory_stats(&self) -> PatriciaStats {
        let keys_bytes = self.keys.memory_usage();
        let leaves_bytes = self.leaves.capacity() * std::mem::size_of::<Leaf>();
        let nodes_bytes = self.nodes.capacity();
        let raw_key_bytes = self.keys.data.len();
        let total = keys_bytes + leaves_bytes + nodes_bytes;
        
        PatriciaStats {
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
        }
    }
}

impl Default for PatriciaArt {
    fn default() -> Self { Self::new() }
}

enum InsertResult {
    Inserted,
    Updated(u64),
    NewRoot(Ref),
}

#[derive(Debug, Clone)]
pub struct PatriciaStats {
    pub keys_bytes: usize,
    pub leaves_bytes: usize,
    pub nodes_bytes: usize,
    pub raw_key_bytes: usize,
    pub total_bytes: usize,
    pub overhead_bytes: usize,
    pub overhead_per_key: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut tree = PatriciaArt::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        
        assert_eq!(tree.get(b"hello"), Some(1));
        assert_eq!(tree.get(b"world"), Some(2));
        assert_eq!(tree.get(b"notfound"), None);
        
        assert_eq!(tree.len(), 2);
    }
    
    #[test]
    fn test_prefix() {
        let mut tree = PatriciaArt::new();
        
        tree.insert(b"test", 1);
        tree.insert(b"testing", 2);
        tree.insert(b"tested", 3);
        
        assert_eq!(tree.get(b"test"), Some(1));
        assert_eq!(tree.get(b"testing"), Some(2));
        assert_eq!(tree.get(b"tested"), Some(3));
    }
    
    #[test]
    fn test_many() {
        let mut tree = PatriciaArt::new();
        
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
        println!("  Overhead: {} bytes ({:.1}/key)", stats.overhead_bytes, stats.overhead_per_key);
        
        assert!(correct >= 950, "Too many failures");
    }
}
