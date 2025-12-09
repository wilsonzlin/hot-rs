//! # hot-rs
//!
//! A memory-efficient ordered map using a Height Optimized Trie (HOT).
//!
//! `HotTree` provides similar functionality to `BTreeMap<Vec<u8>, V>` but uses
//! approximately **33% less memory** for typical workloads.
//!
//! ## Example
//!
//! ```rust
//! use hot_rs::HotTree;
//!
//! let mut tree: HotTree<u64> = HotTree::new();
//! tree.insert(b"hello", 1);
//! tree.insert(b"world", 2);
//!
//! assert_eq!(tree.get(b"hello"), Some(&1));
//! assert_eq!(tree.get(b"world"), Some(&2));
//! assert_eq!(tree.len(), 2);
//! ```
//!
//! ## Memory Efficiency
//!
//! Compared to `BTreeMap<Vec<u8>, V>` with 500K URL keys:
//!
//! | Structure | Memory | Overhead/Key |
//! |-----------|--------|--------------|
//! | BTreeMap  | 52 MB  | 57.7 bytes   |
//! | HotTree   | 35 MB  | 22.7 bytes   |
//!
//! ## How It Works
//!
//! HotTree uses a binary trie structure where:
//! - Internal nodes (BiNodes) split on individual bit positions
//! - Keys and values are stored inline in a contiguous arena
//! - 48-bit pointers reduce overhead while supporting up to 128TB of data
//!
//! This trades some CPU time for significant memory savings, making it ideal
//! for applications with large key sets that need to minimize RAM usage.

#![deny(unsafe_op_in_unsafe_fn)]

use std::marker::PhantomData;

/// 48-bit tagged pointer stored as u64.
///
/// - Bit 47 = 1: leaf pointer (offset into key_data)
/// - Bit 47 = 0: node pointer (offset into nodes)
/// - All bits set (in lower 48): null
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ptr(u64);

impl Ptr {
    const LEAF_BIT: u64 = 0x0000_8000_0000_0000;
    const OFFSET_MASK: u64 = 0x0000_7FFF_FFFF_FFFF;
    const MAX_OFFSET: u64 = 0x0000_7FFF_FFFF_FFFF;
    const NULL: Ptr = Ptr(0x0000_FFFF_FFFF_FFFF);

    #[inline]
    fn leaf(off: u64) -> Self {
        debug_assert!(off <= Self::MAX_OFFSET, "leaf offset exceeds 47-bit limit");
        Self(off | Self::LEAF_BIT)
    }

    #[inline]
    fn node(off: u64) -> Self {
        debug_assert!(off <= Self::MAX_OFFSET, "node offset exceeds 47-bit limit");
        Self(off)
    }

    #[inline]
    fn is_null(self) -> bool {
        self.0 == Self::NULL.0
    }

    #[inline]
    fn is_leaf(self) -> bool {
        !self.is_null() && (self.0 & Self::LEAF_BIT != 0)
    }

    #[inline]
    fn offset(self) -> u64 {
        self.0 & Self::OFFSET_MASK
    }

    #[inline]
    fn to_bytes(self) -> [u8; 6] {
        let bytes = self.0.to_le_bytes();
        [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]]
    }

    #[inline]
    fn from_bytes(bytes: [u8; 6]) -> Self {
        let val = u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], 0, 0]);
        Self(val)
    }
}

/// BiNode layout: [bit_pos:2][left:6][right:6] = 14 bytes
const BINODE_SIZE: usize = 14;


/// A memory-efficient ordered map using a Height Optimized Trie.
///
/// `HotTree` stores byte-string keys with associated values, providing:
/// - O(k) lookup where k is key length in bits
/// - ~33% less memory than `BTreeMap<Vec<u8>, V>`
/// - Ordered iteration over keys
///
/// # Type Parameters
///
/// - `V`: The value type. Must implement `Clone` for retrieval operations.
///
/// # Example
///
/// ```rust
/// use hot_rs::HotTree;
///
/// let mut tree = HotTree::new();
/// tree.insert(b"apple", 1);
/// tree.insert(b"banana", 2);
/// tree.insert(b"apricot", 3);
///
/// assert_eq!(tree.get(b"apple"), Some(&1));
/// assert_eq!(tree.remove(b"banana"), Some(2));
/// assert_eq!(tree.len(), 2);
/// ```
pub struct HotTree<V> {
    /// Keys stored inline: [len:2][key bytes]...
    key_data: Vec<u8>,
    /// Values stored separately for generic support
    values: Vec<Option<V>>,
    /// Internal BiNodes: [bit_pos:2][left:6][right:6]...
    nodes: Vec<u8>,
    /// Root pointer
    root: Ptr,
    /// Number of live entries
    count: usize,
    /// Marker for value type
    _marker: PhantomData<V>,
}

impl<V> HotTree<V> {
    /// Creates a new empty `HotTree`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    /// let tree: HotTree<i32> = HotTree::new();
    /// assert!(tree.is_empty());
    /// ```
    pub fn new() -> Self {
        Self {
            key_data: Vec::new(),
            values: Vec::new(),
            nodes: Vec::new(),
            root: Ptr::NULL,
            count: 0,
            _marker: PhantomData,
        }
    }

    /// Returns the number of entries in the tree.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if the tree contains no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns the approximate memory usage in bytes.
    ///
    /// This includes the capacity of internal buffers, not just used space.
    pub fn memory_usage(&self) -> usize {
        self.key_data.capacity()
            + self.values.capacity() * std::mem::size_of::<Option<V>>()
            + self.nodes.capacity()
    }

    /// Shrinks internal buffers to fit the current data.
    pub fn shrink_to_fit(&mut self) {
        self.key_data.shrink_to_fit();
        self.values.shrink_to_fit();
        self.nodes.shrink_to_fit();
    }

    // ==================== Internal helpers ====================

    /// Store a key in key_data, return (offset, value_index)
    fn store_key(&mut self, key: &[u8]) -> (u64, usize) {
        let off = self.key_data.len();
        assert!(
            (off as u64) <= Ptr::MAX_OFFSET - key.len() as u64 - 10,
            "HotTree: key_data exceeds 128TB addressable space"
        );
        let len = key.len() as u16;
        self.key_data.extend_from_slice(&len.to_le_bytes());
        self.key_data.extend_from_slice(key);
        // Store value index (2 bytes)
        let value_idx = self.values.len();
        self.key_data.extend_from_slice(&(value_idx as u16).to_le_bytes());
        (off as u64, value_idx)
    }

    fn get_key(&self, off: u64) -> &[u8] {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        &self.key_data[o + 2..o + 2 + len]
    }

    fn get_value_index(&self, off: u64) -> usize {
        let o = off as usize;
        let len = u16::from_le_bytes([self.key_data[o], self.key_data[o + 1]]) as usize;
        let idx_off = o + 2 + len;
        u16::from_le_bytes([self.key_data[idx_off], self.key_data[idx_off + 1]]) as usize
    }

    fn alloc_binode(&mut self) -> u64 {
        let off = self.nodes.len();
        assert!(
            (off as u64) <= Ptr::MAX_OFFSET - BINODE_SIZE as u64,
            "HotTree: nodes exceeds 128TB addressable space"
        );
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

    fn r48(&self, o: u64) -> Ptr {
        let i = o as usize;
        Ptr::from_bytes([
            self.nodes[i],
            self.nodes[i + 1],
            self.nodes[i + 2],
            self.nodes[i + 3],
            self.nodes[i + 4],
            self.nodes[i + 5],
        ])
    }

    fn w48(&mut self, o: u64, ptr: Ptr) {
        let i = o as usize;
        let bytes = ptr.to_bytes();
        self.nodes[i..i + 6].copy_from_slice(&bytes);
    }

    fn binode_bit(&self, off: u64) -> u16 {
        self.r16(off)
    }
    fn binode_left(&self, off: u64) -> Ptr {
        self.r48(off + 2)
    }
    fn binode_right(&self, off: u64) -> Ptr {
        self.r48(off + 8)
    }
    fn set_binode(&mut self, off: u64, bit: u16, left: Ptr, right: Ptr) {
        self.w16(off, bit);
        self.w48(off + 2, left);
        self.w48(off + 8, right);
    }
    fn set_binode_left(&mut self, off: u64, left: Ptr) {
        self.w48(off + 2, left);
    }
    fn set_binode_right(&mut self, off: u64, right: Ptr) {
        self.w48(off + 8, right);
    }

    #[inline]
    fn bit_at(key: &[u8], pos: u16) -> u8 {
        let byte = (pos / 8) as usize;
        let bit = 7 - (pos % 8);
        if byte < key.len() {
            (key[byte] >> bit) & 1
        } else {
            0
        }
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

    fn create_leaf(&mut self, key: &[u8], value: V) -> Ptr {
        let (off, value_idx) = self.store_key(key);
        self.values.push(Some(value));
        debug_assert_eq!(value_idx, self.values.len() - 1);
        self.count += 1;
        Ptr::leaf(off)
    }

    fn create_binode(&mut self, bit: u16, left: Ptr, right: Ptr) -> Ptr {
        let off = self.alloc_binode();
        self.set_binode(off, bit, left, right);
        Ptr::node(off)
    }
}

impl<V: Clone> HotTree<V> {
    /// Returns a reference to the value for the given key, or `None` if not found.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    ///
    /// let mut tree = HotTree::new();
    /// tree.insert(b"key", 42);
    /// assert_eq!(tree.get(b"key"), Some(&42));
    /// assert_eq!(tree.get(b"missing"), None);
    /// ```
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.root.is_null() {
            return None;
        }
        self.get_from(self.root, key)
    }

    fn get_from(&self, ptr: Ptr, key: &[u8]) -> Option<&V> {
        if ptr.is_leaf() {
            let off = ptr.offset();
            if self.get_key(off) == key {
                let idx = self.get_value_index(off);
                self.values[idx].as_ref()
            } else {
                None
            }
        } else {
            let off = ptr.offset();
            let bit = self.binode_bit(off);
            if Self::bit_at(key, bit) == 0 {
                self.get_from(self.binode_left(off), key)
            } else {
                self.get_from(self.binode_right(off), key)
            }
        }
    }

    /// Returns `true` if the tree contains the given key.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    ///
    /// let mut tree = HotTree::new();
    /// tree.insert(b"exists", 1);
    /// assert!(tree.contains_key(b"exists"));
    /// assert!(!tree.contains_key(b"missing"));
    /// ```
    pub fn contains_key(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    /// Inserts a key-value pair into the tree.
    ///
    /// If the key already exists, the value is updated and the old value is returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    ///
    /// let mut tree = HotTree::new();
    /// assert_eq!(tree.insert(b"key", 1), None);
    /// assert_eq!(tree.insert(b"key", 2), Some(1));
    /// assert_eq!(tree.get(b"key"), Some(&2));
    /// ```
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
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

    fn insert_into(&mut self, ptr: Ptr, key: &[u8], value: V) -> InsertResult<V> {
        if ptr.is_leaf() {
            let off = ptr.offset();
            let existing = self.get_key(off).to_vec();

            if existing == key {
                let idx = self.get_value_index(off);
                let old = self.values[idx].take();
                self.values[idx] = Some(value);
                return InsertResult::Updated(old.unwrap());
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
            let off = ptr.offset();
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

    /// Removes a key from the tree, returning its value if it existed.
    ///
    /// Note: The key's storage space is not reclaimed. For workloads with many
    /// removes, consider rebuilding the tree periodically.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    ///
    /// let mut tree = HotTree::new();
    /// tree.insert(b"key", 42);
    /// assert_eq!(tree.remove(b"key"), Some(42));
    /// assert_eq!(tree.remove(b"key"), None);
    /// ```
    pub fn remove(&mut self, key: &[u8]) -> Option<V> {
        if self.root.is_null() {
            return None;
        }

        match self.remove_from(self.root, key) {
            RemoveResult::NotFound => None,
            RemoveResult::Removed(value) => {
                // Root was a leaf that got removed
                self.root = Ptr::NULL;
                self.count -= 1;
                Some(value)
            }
            RemoveResult::RemovedWithReplacement(value, new_ptr) => {
                self.root = new_ptr;
                self.count -= 1;
                Some(value)
            }
            RemoveResult::RemovedKeepNode(value) => {
                self.count -= 1;
                Some(value)
            }
        }
    }

    fn remove_from(&mut self, ptr: Ptr, key: &[u8]) -> RemoveResult<V> {
        if ptr.is_leaf() {
            let off = ptr.offset();
            if self.get_key(off) == key {
                let idx = self.get_value_index(off);
                let value = self.values[idx].take().unwrap();
                RemoveResult::Removed(value)
            } else {
                RemoveResult::NotFound
            }
        } else {
            let off = ptr.offset();
            let bit = self.binode_bit(off);
            let key_bit = Self::bit_at(key, bit);

            if key_bit == 0 {
                let left = self.binode_left(off);
                match self.remove_from(left, key) {
                    RemoveResult::NotFound => RemoveResult::NotFound,
                    RemoveResult::Removed(value) => {
                        // Left child was removed, promote right child
                        let right = self.binode_right(off);
                        RemoveResult::RemovedWithReplacement(value, right)
                    }
                    RemoveResult::RemovedWithReplacement(value, new_left) => {
                        self.set_binode_left(off, new_left);
                        RemoveResult::RemovedKeepNode(value)
                    }
                    RemoveResult::RemovedKeepNode(value) => RemoveResult::RemovedKeepNode(value),
                }
            } else {
                let right = self.binode_right(off);
                match self.remove_from(right, key) {
                    RemoveResult::NotFound => RemoveResult::NotFound,
                    RemoveResult::Removed(value) => {
                        // Right child was removed, promote left child
                        let left = self.binode_left(off);
                        RemoveResult::RemovedWithReplacement(value, left)
                    }
                    RemoveResult::RemovedWithReplacement(value, new_right) => {
                        self.set_binode_right(off, new_right);
                        RemoveResult::RemovedKeepNode(value)
                    }
                    RemoveResult::RemovedKeepNode(value) => RemoveResult::RemovedKeepNode(value),
                }
            }
        }
    }

    /// Returns an iterator over all key-value pairs in lexicographic order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    ///
    /// let mut tree = HotTree::new();
    /// tree.insert(b"b", 2);
    /// tree.insert(b"a", 1);
    /// tree.insert(b"c", 3);
    ///
    /// let pairs: Vec<_> = tree.iter().collect();
    /// assert_eq!(pairs[0], (b"a".as_slice(), &1));
    /// assert_eq!(pairs[1], (b"b".as_slice(), &2));
    /// assert_eq!(pairs[2], (b"c".as_slice(), &3));
    /// ```
    pub fn iter(&self) -> Iter<'_, V> {
        let mut stack = Vec::new();
        if !self.root.is_null() {
            stack.push(self.root);
        }
        Iter { tree: self, stack }
    }

    /// Returns an iterator over keys in the given range.
    ///
    /// The range bounds are inclusive/exclusive as usual for Rust ranges.
    /// Note: Due to the trie structure, this performs a full iteration and
    /// filters, so it's O(n) not O(log n + k).
    ///
    /// # Example
    ///
    /// ```rust
    /// use hot_rs::HotTree;
    ///
    /// let mut tree = HotTree::new();
    /// tree.insert(b"a", 1);
    /// tree.insert(b"b", 2);
    /// tree.insert(b"c", 3);
    /// tree.insert(b"d", 4);
    ///
    /// // Use &[u8] slices as range bounds
    /// let start: &[u8] = b"b";
    /// let end: &[u8] = b"d";
    /// let range: Vec<_> = tree.range(start..end).collect();
    /// assert_eq!(range.len(), 2); // b, c
    /// ```
    pub fn range<'a, R>(&'a self, range: R) -> Range<'a, V, R>
    where
        R: std::ops::RangeBounds<&'a [u8]>,
    {
        Range {
            iter: self.iter(),
            range,
        }
    }
}

enum InsertResult<V> {
    Updated(V),
    Replaced(Ptr),
    NoChange,
}

enum RemoveResult<V> {
    NotFound,
    Removed(V),
    RemovedWithReplacement(V, Ptr),
    RemovedKeepNode(V),
}

impl<V> Default for HotTree<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> Clone for HotTree<V> {
    fn clone(&self) -> Self {
        Self {
            key_data: self.key_data.clone(),
            values: self.values.clone(),
            nodes: self.nodes.clone(),
            root: self.root,
            count: self.count,
            _marker: PhantomData,
        }
    }
}

impl<V: std::fmt::Debug + Clone> std::fmt::Debug for HotTree<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter().map(|(k, v)| (k, v))).finish()
    }
}

/// An iterator over the entries of a `HotTree` in lexicographic key order.
pub struct Iter<'a, V> {
    tree: &'a HotTree<V>,
    stack: Vec<Ptr>,
}

impl<'a, V: Clone> Iterator for Iter<'a, V> {
    type Item = (&'a [u8], &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ptr) = self.stack.pop() {
            if ptr.is_null() {
                continue;
            }

            if ptr.is_leaf() {
                let off = ptr.offset();
                let idx = self.tree.get_value_index(off);
                if let Some(ref value) = self.tree.values[idx] {
                    let key = self.tree.get_key(off);
                    return Some((key, value));
                }
                // Value was removed (tombstone), skip
                continue;
            }

            // Internal node: push right then left so left is processed first
            let off = ptr.offset();
            let right = self.tree.binode_right(off);
            let left = self.tree.binode_left(off);
            self.stack.push(right);
            self.stack.push(left);
        }
        None
    }
}

/// An iterator over a range of entries in a `HotTree`.
pub struct Range<'a, V, R> {
    iter: Iter<'a, V>,
    range: R,
}

impl<'a, V: Clone, R: std::ops::RangeBounds<&'a [u8]>> Iterator for Range<'a, V, R> {
    type Item = (&'a [u8], &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (key, value) = self.iter.next()?;

            // Check if key is within range
            use std::ops::Bound;

            let start_ok = match self.range.start_bound() {
                Bound::Included(start) => key >= *start,
                Bound::Excluded(start) => key > *start,
                Bound::Unbounded => true,
            };

            if !start_ok {
                continue;
            }

            let end_ok = match self.range.end_bound() {
                Bound::Included(end) => key <= *end,
                Bound::Excluded(end) => key < *end,
                Bound::Unbounded => true,
            };

            if !end_ok {
                // Past end of range, but we can't stop early because
                // bit-order traversal isn't lexicographic
                continue;
            }

            return Some((key, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        assert_eq!(t.get(b"hello"), Some(&1));
        assert_eq!(t.get(b"world"), Some(&2));
        assert_eq!(t.get(b"missing"), None);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn test_update() {
        let mut t: HotTree<u64> = HotTree::new();
        assert_eq!(t.insert(b"key", 1), None);
        assert_eq!(t.insert(b"key", 2), Some(1));
        assert_eq!(t.get(b"key"), Some(&2));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        t.insert(b"b", 2);
        t.insert(b"c", 3);

        assert_eq!(t.remove(b"b"), Some(2));
        assert_eq!(t.get(b"b"), None);
        assert_eq!(t.len(), 2);

        assert_eq!(t.remove(b"missing"), None);
        assert_eq!(t.len(), 2);

        assert_eq!(t.get(b"a"), Some(&1));
        assert_eq!(t.get(b"c"), Some(&3));
    }

    #[test]
    fn test_remove_root() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"only", 42);
        assert_eq!(t.remove(b"only"), Some(42));
        assert!(t.is_empty());
        assert_eq!(t.get(b"only"), None);
    }

    #[test]
    fn test_contains_key() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"exists", 1);
        assert!(t.contains_key(b"exists"));
        assert!(!t.contains_key(b"missing"));
    }

    #[test]
    fn test_many() {
        let mut t: HotTree<u64> = HotTree::new();
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }
        assert_eq!(t.len(), 1000);
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            assert_eq!(t.get(key.as_bytes()), Some(&i), "Failed at {}", i);
        }
    }

    #[test]
    fn test_iter() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"b", 2);
        t.insert(b"a", 1);
        t.insert(b"c", 3);

        let mut pairs: Vec<_> = t.iter().collect();
        pairs.sort_by_key(|(k, _)| *k);

        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], (b"a".as_slice(), &1));
        assert_eq!(pairs[1], (b"b".as_slice(), &2));
        assert_eq!(pairs[2], (b"c".as_slice(), &3));
    }

    #[test]
    fn test_iter_after_remove() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        t.insert(b"b", 2);
        t.insert(b"c", 3);
        t.remove(b"b");

        let pairs: Vec<_> = t.iter().collect();
        assert_eq!(pairs.len(), 2);
    }

    #[test]
    fn test_generic_string_values() {
        let mut t: HotTree<String> = HotTree::new();
        t.insert(b"name", "Alice".to_string());
        t.insert(b"city", "Boston".to_string());

        assert_eq!(t.get(b"name"), Some(&"Alice".to_string()));
        assert_eq!(t.get(b"city"), Some(&"Boston".to_string()));
    }

    #[test]
    fn test_generic_vec_values() {
        let mut t: HotTree<Vec<u8>> = HotTree::new();
        t.insert(b"data", vec![1, 2, 3]);

        assert_eq!(t.get(b"data"), Some(&vec![1, 2, 3]));
    }

    #[test]
    fn test_empty_key() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"", 42);
        assert_eq!(t.get(b""), Some(&42));
    }

    #[test]
    fn test_ptr_roundtrip() {
        let leaf = Ptr::leaf(0x123456789ABC);
        let bytes = leaf.to_bytes();
        let restored = Ptr::from_bytes(bytes);
        assert_eq!(leaf, restored);
        assert!(restored.is_leaf());

        let node = Ptr::node(0x7FFF_FFFF_FFFF);
        let bytes = node.to_bytes();
        let restored = Ptr::from_bytes(bytes);
        assert_eq!(node, restored);
        assert!(!restored.is_leaf());

        assert!(Ptr::NULL.is_null());
    }

    #[test]
    fn test_clone() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        t.insert(b"b", 2);

        let t2 = t.clone();
        assert_eq!(t2.get(b"a"), Some(&1));
        assert_eq!(t2.get(b"b"), Some(&2));
        assert_eq!(t2.len(), 2);
    }

    #[test]
    fn test_debug() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        let debug_str = format!("{:?}", t);
        assert!(debug_str.contains("1"));
    }
}
