//! HOT with inline values - targeting minimum overhead
//!
//! Key insight: Store values directly in key_data alongside keys
//! This eliminates the separate leaves array overhead.
//!
//! Layout: key_data = [...[len:2][key bytes][value:8]...]
//! Overhead per key: 2 (len) + tree structure
//! With compound nodes: ~12 B/K total

/// Pointer: 4 bytes, high bit = leaf flag
/// Leaf: points directly into key_data
/// Node: points into nodes array
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ptr(u32);

impl Ptr {
    const LEAF_BIT: u32 = 0x8000_0000;
    const NULL: Ptr = Ptr(u32::MAX);
    
    #[inline] fn leaf(off: u32) -> Self { Self(off | Self::LEAF_BIT) }
    #[inline] fn node(off: u32) -> Self { Self(off) }
    #[inline] fn is_null(self) -> bool { self.0 == u32::MAX }
    #[inline] fn is_leaf(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT != 0) }
    #[inline] fn is_node(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT == 0) }
    #[inline] fn leaf_off(self) -> u32 { self.0 & !Self::LEAF_BIT }
    #[inline] fn node_off(self) -> u32 { self.0 }
}

const BINODE_SIZE: usize = 10;

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

    fn store_entry(&mut self, key: &[u8], value: u64) -> u32 {
        let off = self.key_data.len() as u32;
        let len = key.len() as u16;
        self.key_data.extend_from_slice(&len.to_le_bytes());
        self.key_data.extend_from_slice(key);
        self.key_data.extend_from_slice(&value.to_le_bytes());
        off
    }

    fn get_key(&self, off: u32) -> &[u8] {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        &self.key_data[o + 2..o + 2 + len]
    }

    fn get_value(&self, off: u32) -> u64 {
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

    fn set_value(&mut self, off: u32, value: u64) {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        let val_off = o + 2 + len;
        self.key_data[val_off..val_off + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn alloc_binode(&mut self) -> u32 {
        let off = self.nodes.len() as u32;
        self.nodes.resize(self.nodes.len() + BINODE_SIZE, 0);
        off
    }

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

    fn binode_bit(&self, off: u32) -> u16 { self.r16(off) }
    fn binode_left(&self, off: u32) -> Ptr { Ptr(self.r32(off + 2)) }
    fn binode_right(&self, off: u32) -> Ptr { Ptr(self.r32(off + 6)) }
    fn set_binode(&mut self, off: u32, bit: u16, left: Ptr, right: Ptr) {
        self.w16(off, bit);
        self.w32(off + 2, left.0);
        self.w32(off + 6, right.0);
    }
    fn set_binode_left(&mut self, off: u32, left: Ptr) { self.w32(off + 2, left.0); }
    fn set_binode_right(&mut self, off: u32, right: Ptr) { self.w32(off + 6, right.0); }

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
}
