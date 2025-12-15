//! # hot-rs
//!
//! A memory-efficient ordered map using Height Optimized Trie (HOT).
//!
//! Based on "HOT: A Height Optimized Trie Index for Main-Memory Database Systems"
//! (SIGMOD 2018, Binna et al.)
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
//! ```

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::HashMap;
use std::marker::PhantomData;

/// Statistics about tree structure
#[derive(Default, Debug)]
pub struct TreeStats {
    pub node_count: usize,
    pub leaf_count: usize,
    pub max_depth: usize,
    pub both_children_nodes: usize,  // Potential N4 candidates
    pub one_child_node: usize,
    pub matching_disc_children: usize,  // Children with matching discriminators
}

/// Insert profiling stats
#[derive(Default, Debug)]
pub struct InsertStats {
    pub total_path_steps: u64,
    pub total_splices: u64,
    pub total_sample_key_steps: u64,
    pub insert_count: u64,
}

// =============================================================================
// Configuration
// =============================================================================

const MIN_PREFIX_LEN: usize = 4;   // Minimum prefix length to consider
const MAX_PREFIX_LEN: usize = 128; // Maximum prefix length
const MAX_PREFIXES: usize = 65535; // Maximum unique prefixes (u16 max - 1)

// =============================================================================
// Pointer type
// =============================================================================

/// Pointer: 32-bit tagged
/// Bit 31 = 1: Leaf INDEX - index into leaf_offsets array
/// Bit 31 = 0: Node INDEX - index into node_offsets array
/// Special: 0xFFFF_FFFF = NULL
#[derive(Clone, Copy, PartialEq, Eq)]
struct Ptr(u32);

impl Ptr {
    const LEAF_BIT: u32 = 0x8000_0000;
    const INDEX_MASK: u32 = 0x7FFF_FFFF; // 31 bits for index
    const NULL: Ptr = Ptr(0xFFFF_FFFF);

    #[inline]
    fn leaf(idx: u32) -> Self { Self(idx | Self::LEAF_BIT) }

    #[inline]
    fn node(idx: u32) -> Self { Self(idx) }

    #[inline]
    fn is_null(self) -> bool { self.0 == Self::NULL.0 }

    #[inline]
    fn is_leaf(self) -> bool { !self.is_null() && (self.0 & Self::LEAF_BIT) != 0 }

    /// For leaves: returns leaf index (use with leaf_offsets to get byte offset)
    #[inline]
    fn leaf_index(self) -> u32 { self.0 & Self::INDEX_MASK }

    /// For nodes: returns node index (use with node_offsets to get byte offset)
    #[inline]
    fn node_index(self) -> u32 { self.0 & Self::INDEX_MASK }
}

// =============================================================================
// Node Arena - Packed storage for multiple node types
// =============================================================================
//
// Node types (tag in high 2 bits of first byte):
// - Type 0 (BI): 1 discriminator, 2 children - 10 bytes [d0:2][left:4][right:4]
// - Type 1 (N4): 2 discriminators, 4 children - 20 bytes [d0:2][d1:2][c0-c3:16]
// - Type 2 (N8): 3 discriminators, 8 children - 38 bytes [d0:2][d1:2][d2:2][c0-c7:32]
//
// Ptr format for node pointers: byte offset into node arena (NOT index)

/// Node type tags (for future compound nodes)
#[allow(dead_code)]
const NODE_BI: u8 = 0;  // Binary node
#[allow(dead_code)]
const NODE_N4: u8 = 1;  // 4-way node
#[allow(dead_code)]
const NODE_N8: u8 = 2;  // 8-way node

/// Node sizes
/// Binary nodes: [disc:2][left:4][right:4] = 10 bytes (no type tag)
/// Tagged Binary: [tag:1][disc:2][left:4][right:4] = 11 bytes
/// N4: [tag:1][d0:2][d1:2][c0-c3:16] = 21 bytes
/// N8: [tag:1][d0:2][d1:2][d2:2][c0-c7:32] = 39 bytes
#[allow(dead_code)]
const SIZE_BI: usize = 2 + 4 + 4;           // 10 bytes
#[allow(dead_code)]
const SIZE_BI_TAGGED: usize = 1 + 2 + 4 + 4; // 11 bytes
#[allow(dead_code)]
const SIZE_N4: usize = 1 + 4 + 16;          // 21 bytes
#[allow(dead_code)]
const SIZE_N8: usize = 1 + 6 + 32;          // 39 bytes

/// Node arena for packed storage
#[derive(Clone)]
struct NodeArena {
    data: Vec<u8>,
}

impl NodeArena {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn capacity(&self) -> usize {
        self.data.capacity()
    }

    fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
    }

    // =========================================================================
    // Binary Node - 10 bytes: [disc:2][left:4][right:4]
    // =========================================================================

    fn alloc_bi(&mut self, disc: u16, left: Ptr, right: Ptr) -> u32 {
        let off = self.data.len() as u32;
        self.data.reserve(SIZE_BI);
        self.data.extend_from_slice(&disc.to_le_bytes());
        self.data.extend_from_slice(&left.0.to_le_bytes());
        self.data.extend_from_slice(&right.0.to_le_bytes());
        off
    }

    #[inline]
    fn get_bi_disc(&self, off: u32) -> u16 {
        let o = off as usize;
        unsafe {
            let ptr = self.data.as_ptr().add(o);
            u16::from_le_bytes([*ptr, *ptr.add(1)])
        }
    }

    #[inline]
    fn get_bi_left(&self, off: u32) -> Ptr {
        let o = off as usize + 2;
        unsafe {
            let ptr = self.data.as_ptr().add(o);
            Ptr(u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]))
        }
    }

    #[inline]
    fn get_bi_right(&self, off: u32) -> Ptr {
        let o = off as usize + 6;
        unsafe {
            let ptr = self.data.as_ptr().add(o);
            Ptr(u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]))
        }
    }

    #[inline]
    fn set_bi_left(&mut self, off: u32, ptr: Ptr) {
        let o = off as usize + 2;
        let bytes = ptr.0.to_le_bytes();
        unsafe {
            let dest = self.data.as_mut_ptr().add(o);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, 4);
        }
    }

    #[inline]
    fn set_bi_right(&mut self, off: u32, ptr: Ptr) {
        let o = off as usize + 6;
        let bytes = ptr.0.to_le_bytes();
        unsafe {
            let dest = self.data.as_mut_ptr().add(o);
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, 4);
        }
    }

    // =========================================================================
    // Tagged Binary Node - 11 bytes: [tag:1][disc:2][left:4][right:4]
    // =========================================================================

    #[allow(dead_code)]
    fn alloc_bi_tagged(&mut self, disc: u16, left: Ptr, right: Ptr) -> u32 {
        let off = self.data.len() as u32;
        self.data.reserve(SIZE_BI_TAGGED);
        self.data.push(NODE_BI);
        self.data.extend_from_slice(&disc.to_le_bytes());
        self.data.extend_from_slice(&left.0.to_le_bytes());
        self.data.extend_from_slice(&right.0.to_le_bytes());
        off
    }

    #[allow(dead_code)]
    #[inline]
    fn get_bi_tagged_disc(&self, off: u32) -> u16 {
        let o = off as usize + 1;
        unsafe {
            let ptr = self.data.as_ptr().add(o);
            u16::from_le_bytes([*ptr, *ptr.add(1)])
        }
    }

    #[allow(dead_code)]
    #[inline]
    fn get_bi_tagged_left(&self, off: u32) -> Ptr {
        let o = off as usize + 3;
        unsafe {
            let ptr = self.data.as_ptr().add(o);
            Ptr(u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]))
        }
    }

    #[allow(dead_code)]
    #[inline]
    fn get_bi_tagged_right(&self, off: u32) -> Ptr {
        let o = off as usize + 7;
        unsafe {
            let ptr = self.data.as_ptr().add(o);
            Ptr(u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]))
        }
    }

    // =========================================================================
    // Node4 (Type 1) - 2 discriminators, 4 children - 21 bytes
    // NOTE: Currently unused - reserved for future compaction support
    // =========================================================================

    #[allow(dead_code)]
    fn alloc_n4(&mut self, d0: u16, d1: u16, children: [Ptr; 4]) -> u32 {
        let off = self.data.len() as u32;
        self.data.push(NODE_N4);
        self.data.extend_from_slice(&d0.to_le_bytes());
        self.data.extend_from_slice(&d1.to_le_bytes());
        for c in &children {
            self.data.extend_from_slice(&c.0.to_le_bytes());
        }
        off
    }

    #[allow(dead_code)]
    #[inline]
    fn get_n4_disc(&self, off: u32) -> (u16, u16) {
        let o = off as usize + 1;
        let d0 = u16::from_le_bytes([self.data[o], self.data[o + 1]]);
        let d1 = u16::from_le_bytes([self.data[o + 2], self.data[o + 3]]);
        (d0, d1)
    }

    #[allow(dead_code)]
    #[inline]
    fn get_n4_child(&self, off: u32, idx: usize) -> Ptr {
        debug_assert!(idx < 4);
        let o = off as usize + 5 + idx * 4;
        Ptr(u32::from_le_bytes([
            self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3]
        ]))
    }

    #[allow(dead_code)]
    #[inline]
    fn set_n4_child(&mut self, off: u32, idx: usize, ptr: Ptr) {
        debug_assert!(idx < 4);
        let o = off as usize + 5 + idx * 4;
        self.data[o..o + 4].copy_from_slice(&ptr.0.to_le_bytes());
    }

    // =========================================================================
    // Node8 (Type 2) - 3 discriminators, 8 children
    // NOTE: Currently unused - reserved for future compaction support
    // =========================================================================

    #[allow(dead_code)]
    fn alloc_n8(&mut self, d0: u16, d1: u16, d2: u16, children: [Ptr; 8]) -> u32 {
        let off = self.data.len() as u32;
        self.data.push(NODE_N8);
        self.data.extend_from_slice(&d0.to_le_bytes());
        self.data.extend_from_slice(&d1.to_le_bytes());
        self.data.extend_from_slice(&d2.to_le_bytes());
        for c in &children {
            self.data.extend_from_slice(&c.0.to_le_bytes());
        }
        off
    }

    #[allow(dead_code)]
    #[inline]
    fn get_n8_disc(&self, off: u32) -> (u16, u16, u16) {
        let o = off as usize + 1;
        let d0 = u16::from_le_bytes([self.data[o], self.data[o + 1]]);
        let d1 = u16::from_le_bytes([self.data[o + 2], self.data[o + 3]]);
        let d2 = u16::from_le_bytes([self.data[o + 4], self.data[o + 5]]);
        (d0, d1, d2)
    }

    #[allow(dead_code)]
    #[inline]
    fn get_n8_child(&self, off: u32, idx: usize) -> Ptr {
        debug_assert!(idx < 8);
        let o = off as usize + 7 + idx * 4;
        Ptr(u32::from_le_bytes([
            self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3]
        ]))
    }

    #[allow(dead_code)]
    #[inline]
    fn set_n8_child(&mut self, off: u32, idx: usize, ptr: Ptr) {
        debug_assert!(idx < 8);
        let o = off as usize + 7 + idx * 4;
        self.data[o..o + 4].copy_from_slice(&ptr.0.to_le_bytes());
    }

    // =========================================================================
    // Generic accessors
    // =========================================================================

    #[allow(dead_code)]
    #[inline]
    fn node_type(&self, off: u32) -> u8 {
        self.data[off as usize]
    }
}

// =============================================================================
// HotTree with adaptive prefix compression
// =============================================================================

/// A memory-efficient ordered map using Height Optimized Trie.
///
/// Features:
/// - Arena-based leaf storage with prefix compression
/// - Packed node arena with multiple node types
/// - Adaptive prefix learning from natural delimiters
/// - ZST value optimization
pub struct HotTree<V> {
    // === Prefix compression ===
    /// Prefix pool: contiguous storage of all prefixes
    prefix_pool: Vec<u8>,
    /// Offset of each prefix in pool (prefix_id -> offset)
    prefix_offsets: Vec<u32>,
    /// Map from prefix hash to prefix_id for fast lookup
    prefix_hash: HashMap<u64, u16>,

    // === Leaf storage ===
    /// Leaf arena: [prefix_id:2][suffix_len:1-3][suffix...][value_idx:4]
    leaves: Vec<u8>,
    /// Maps leaf index -> byte offset in leaves arena (u64 to support > 4GB)
    leaf_offsets: Vec<u64>,

    // === Values ===
    values: Vec<Option<V>>,

    // === Trie structure ===
    /// Packed node arena (multiple node types)
    nodes: NodeArena,
    /// Maps node index -> byte offset in nodes arena (u64 to support > 4GB)
    node_offsets: Vec<u64>,
    root: Ptr,
    count: usize,
    /// True after compact() is called - nodes use type tags
    compacted: bool,
    /// Max tree depth seen during inserts
    max_depth_seen: usize,

    _marker: PhantomData<V>,
}

impl<V> HotTree<V> {
    pub fn new() -> Self {
        let mut tree = Self {
            prefix_pool: Vec::new(),
            prefix_offsets: Vec::new(),
            prefix_hash: HashMap::new(),
            leaves: Vec::new(),
            leaf_offsets: Vec::new(),
            values: Vec::new(),
            nodes: NodeArena::new(),
            node_offsets: Vec::new(),
            root: Ptr::NULL,
            count: 0,
            compacted: false,
            max_depth_seen: 0,
            _marker: PhantomData,
        };
        // Register empty prefix as ID 0
        tree.register_prefix(&[]);
        tree
    }

    pub fn max_depth(&self) -> usize { self.max_depth_seen }

    #[inline]
    pub fn len(&self) -> usize { self.count }

    #[inline]
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn memory_usage(&self) -> usize {
        self.prefix_pool.capacity()
            + self.prefix_offsets.capacity() * 4
            + self.prefix_hash.capacity() * 16
            + self.leaves.capacity()
            + self.leaf_offsets.capacity() * 8  // Vec<u64>
            + self.values.capacity() * std::mem::size_of::<Option<V>>()
            + self.nodes.capacity()
            + self.node_offsets.capacity() * 8  // Vec<u64>
    }

    pub fn memory_breakdown(&self) -> (usize, usize, usize, usize) {
        let prefix_mem = self.prefix_pool.capacity()
            + self.prefix_offsets.capacity() * 4
            + self.prefix_hash.capacity() * 16;
        (
            prefix_mem,
            self.leaves.capacity() + self.leaf_offsets.capacity() * 8,
            self.values.capacity() * std::mem::size_of::<Option<V>>(),
            self.nodes.capacity() + self.node_offsets.capacity() * 8,
        )
    }

    /// Get leaves arena exact len and capacity
    pub fn leaves_stats(&self) -> (usize, usize) {
        (self.leaves.len(), self.leaves.capacity())
    }

    /// Get prefix pool stats
    pub fn prefix_stats(&self) -> (usize, usize, usize) {
        (self.prefix_offsets.len(), self.prefix_pool.len(), self.prefix_hash.len())
    }

    pub fn shrink_to_fit(&mut self) {
        self.prefix_pool.shrink_to_fit();
        self.prefix_offsets.shrink_to_fit();
        self.prefix_hash.shrink_to_fit();
        self.leaves.shrink_to_fit();
        self.leaf_offsets.shrink_to_fit();
        self.values.shrink_to_fit();
        self.nodes.shrink_to_fit();
        self.node_offsets.shrink_to_fit();
    }

    /// Analyze tree structure for compaction opportunities
    pub fn analyze_structure(&self) -> TreeStats {
        let mut stats = TreeStats::default();
        if !self.root.is_null() {
            self.analyze_node(self.root, 0, &mut stats);
        }
        stats
    }

    fn analyze_node(&self, ptr: Ptr, depth: usize, stats: &mut TreeStats) {
        stats.max_depth = stats.max_depth.max(depth);

        if ptr.is_leaf() {
            stats.leaf_count += 1;
            return;
        }

        let (_, left, right) = self.get_node(ptr.node_index());
        stats.node_count += 1;

        let left_is_node = !left.is_null() && !left.is_leaf();
        let right_is_node = !right.is_null() && !right.is_leaf();

        // Check for compaction opportunities
        if left_is_node && right_is_node {
            stats.both_children_nodes += 1;
            // Check if discriminators match
            let (d_left, _, _) = self.get_node(left.node_index());
            let (d_right, _, _) = self.get_node(right.node_index());
            if d_left == d_right {
                stats.matching_disc_children += 1;
            }
        } else if left_is_node || right_is_node {
            stats.one_child_node += 1;
        }

        if !left.is_null() {
            self.analyze_node(left, depth + 1, stats);
        }
        if !right.is_null() {
            self.analyze_node(right, depth + 1, stats);
        }
    }

    /// Compact the tree by converting chains of binary nodes into compound nodes.
    /// NOTE: Compaction is currently disabled due to index-based pointer refactoring.
    /// Returns 0 (no N4 nodes created).
    pub fn compact(&mut self) -> usize {
        // Compaction disabled - would require reworking compound pointer scheme
        // to work with node indices instead of byte offsets
        0
    }

    /// FNV-1a hash for prefix lookup
    #[inline]
    fn hash_prefix(prefix: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for &byte in prefix {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    // =========================================================================
    // Prefix compression
    // =========================================================================

    /// Extract natural prefix from key (up to delimiter)
    fn extract_natural_prefix(key: &[u8]) -> &[u8] {
        if key.len() < MIN_PREFIX_LEN {
            return &[];
        }

        // Look for delimiter within acceptable range
        let max_check = key.len().min(MAX_PREFIX_LEN);
        for i in MIN_PREFIX_LEN..max_check {
            let b = key[i];
            // Natural delimiters: / : \ (common in URLs, paths, URIs)
            if b == b'/' || b == b':' || b == b'\\' {
                return &key[..=i]; // Include delimiter
            }
        }
        &[]
    }

    /// Register a prefix, returns its ID
    fn register_prefix(&mut self, prefix: &[u8]) -> u16 {
        let hash = Self::hash_prefix(prefix);

        // Check if already exists
        if let Some(&id) = self.prefix_hash.get(&hash) {
            // Verify it's actually the same prefix (handle hash collisions)
            let stored = self.get_prefix(id);
            if stored == prefix {
                return id;
            }
            // Hash collision - fall back to empty prefix to be safe
            return 0;
        }

        if self.prefix_offsets.len() >= MAX_PREFIXES {
            return 0; // Fall back to empty prefix
        }

        let id = self.prefix_offsets.len() as u16;

        // Store offset and prefix bytes
        let offset = self.prefix_pool.len() as u32;
        self.prefix_offsets.push(offset);
        self.prefix_pool.extend_from_slice(prefix);

        self.prefix_hash.insert(hash, id);
        id
    }

    /// Get or create prefix for a key
    fn get_or_create_prefix(&mut self, key: &[u8]) -> (u16, usize) {
        let natural = Self::extract_natural_prefix(key);
        if natural.is_empty() {
            return (0, 0); // Empty prefix
        }

        let hash = Self::hash_prefix(natural);

        // Check if prefix exists
        if let Some(&id) = self.prefix_hash.get(&hash) {
            let stored = self.get_prefix(id);
            if stored == natural {
                return (id, natural.len());
            }
            // Hash collision - use empty prefix
            return (0, 0);
        }

        // Register new prefix
        let id = self.register_prefix(natural);
        (id, if id == 0 { 0 } else { natural.len() })
    }

    /// Get prefix bytes for a prefix ID (O(1) lookup)
    #[inline]
    fn get_prefix(&self, id: u16) -> &[u8] {
        let idx = id as usize;
        if idx >= self.prefix_offsets.len() {
            return &[];
        }

        let start = self.prefix_offsets[idx] as usize;
        let end = if idx + 1 < self.prefix_offsets.len() {
            self.prefix_offsets[idx + 1] as usize
        } else {
            self.prefix_pool.len()
        };

        &self.prefix_pool[start..end]
    }

    // =========================================================================
    // Leaf operations
    // =========================================================================

    /// Store leaf with prefix compression
    /// Format: [prefix_id:2][suffix_len:1-3][suffix...][value_idx:3]
    ///
    /// suffix_len encoding:
    /// - If < 255: [len:1]
    /// - If >= 255: [0xFF][len:2]
    ///
    /// Returns leaf INDEX (not byte offset) - use with leaf_offsets to get byte offset.
    fn store_leaf(&mut self, key: &[u8]) -> u32 {
        let (prefix_id, prefix_len) = self.get_or_create_prefix(key);
        let suffix = &key[prefix_len..];

        // Record byte offset and get leaf index
        let byte_offset = self.leaves.len() as u64;
        let leaf_idx = self.leaf_offsets.len() as u32;

        // Check for leaf index overflow (31 bits = 2 billion leaves)
        if leaf_idx > Ptr::INDEX_MASK {
            panic!("LEAF INDEX OVERFLOW: {} leaves exceeds max {}", leaf_idx, Ptr::INDEX_MASK);
        }

        self.leaf_offsets.push(byte_offset);

        // Store prefix_id (2 bytes)
        self.leaves.extend_from_slice(&prefix_id.to_le_bytes());

        // Store suffix_len (variable length - 1 byte for < 255, 3 bytes for >= 255)
        let suffix_len = suffix.len();
        if suffix_len < 255 {
            self.leaves.push(suffix_len as u8);
        } else {
            self.leaves.push(0xFF);
            self.leaves.extend_from_slice(&(suffix_len as u16).to_le_bytes());
        }

        // Store suffix
        self.leaves.extend_from_slice(suffix);

        // Store value_idx (3 bytes) if not ZST - supports up to 16M values
        if std::mem::size_of::<V>() > 0 {
            let value_idx = self.values.len() as u32;
            self.leaves.push(value_idx as u8);
            self.leaves.push((value_idx >> 8) as u8);
            self.leaves.push((value_idx >> 16) as u8);
        }

        leaf_idx
    }

    /// Read suffix_len and return (suffix_len, bytes_consumed_for_header)
    #[inline]
    fn read_suffix_len(&self, off: usize) -> (usize, usize) {
        let first = self.leaves[off];
        if first < 255 {
            (first as usize, 1)
        } else {
            let len = u16::from_le_bytes([self.leaves[off + 1], self.leaves[off + 2]]);
            (len as usize, 3)
        }
    }

    /// Reconstruct full key from leaf index
    fn get_leaf_key(&self, leaf_idx: u32) -> Vec<u8> {
        debug_assert!(
            (leaf_idx as usize) < self.leaf_offsets.len(),
            "get_leaf_key: invalid leaf_idx {} (max {}), raw ptr value: 0x{:08X}",
            leaf_idx, self.leaf_offsets.len(), leaf_idx
        );
        let o = self.leaf_offsets[leaf_idx as usize] as usize;
        let prefix_id = u16::from_le_bytes([self.leaves[o], self.leaves[o + 1]]);
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let suffix_start = o + 2 + slen_bytes;
        let suffix = &self.leaves[suffix_start..suffix_start + suffix_len];

        let prefix = self.get_prefix(prefix_id);
        let mut key = Vec::with_capacity(prefix.len() + suffix.len());
        key.extend_from_slice(prefix);
        key.extend_from_slice(suffix);
        key
    }

    /// Get suffix directly (for bit operations within suffix)
    #[allow(dead_code)]
    fn get_leaf_suffix(&self, leaf_idx: u32) -> (u16, &[u8]) {
        let o = self.leaf_offsets[leaf_idx as usize] as usize;
        let prefix_id = u16::from_le_bytes([self.leaves[o], self.leaves[o + 1]]);
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let suffix_start = o + 2 + slen_bytes;
        (prefix_id, &self.leaves[suffix_start..suffix_start + suffix_len])
    }

    fn get_leaf_value_idx(&self, leaf_idx: u32) -> usize {
        if std::mem::size_of::<V>() == 0 {
            return 0;
        }
        let o = self.leaf_offsets[leaf_idx as usize] as usize;
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let val_off = o + 2 + slen_bytes + suffix_len;
        // Read 3-byte value_idx (little-endian)
        (self.leaves[val_off] as usize)
            | ((self.leaves[val_off + 1] as usize) << 8)
            | ((self.leaves[val_off + 2] as usize) << 16)
    }

    // =========================================================================
    // Node operations
    // =========================================================================

    /// Allocate a binary node, returns node INDEX (not byte offset)
    fn alloc_node(&mut self, disc: u16, left: Ptr, right: Ptr) -> u32 {
        let byte_off = self.nodes.alloc_bi(disc, left, right);
        let node_idx = self.node_offsets.len() as u32;

        // Check for node index overflow (31 bits = 2 billion nodes)
        if node_idx > Ptr::INDEX_MASK {
            panic!("NODE INDEX OVERFLOW: {} nodes exceeds max {}", node_idx, Ptr::INDEX_MASK);
        }

        self.node_offsets.push(byte_off as u64);
        node_idx
    }

    /// Get binary node data (disc, left, right) by node index
    #[inline]
    fn get_node(&self, node_idx: u32) -> (u16, Ptr, Ptr) {
        let off = self.node_offsets[node_idx as usize] as u32;
        (
            self.nodes.get_bi_disc(off),
            self.nodes.get_bi_left(off),
            self.nodes.get_bi_right(off),
        )
    }

    #[inline]
    fn set_node_left(&mut self, node_idx: u32, ptr: Ptr) {
        let off = self.node_offsets[node_idx as usize] as u32;
        self.nodes.set_bi_left(off, ptr);
    }

    #[inline]
    fn set_node_right(&mut self, node_idx: u32, ptr: Ptr) {
        let off = self.node_offsets[node_idx as usize] as u32;
        self.nodes.set_bi_right(off, ptr);
    }

    // =========================================================================
    // Bit operations
    // =========================================================================

    #[inline]
    fn bit_at(key: &[u8], pos: u16) -> u8 {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);
        if byte_idx < key.len() {
            (key[byte_idx] >> bit_idx) & 1
        } else {
            0
        }
    }

    fn first_diff_bit(a: &[u8], b: &[u8]) -> Option<u16> {
        let max_len = a.len().max(b.len());
        for i in 0..max_len {
            let ab = a.get(i).copied().unwrap_or(0);
            let bb = b.get(i).copied().unwrap_or(0);
            if ab != bb {
                let xor = ab ^ bb;
                return Some(i as u16 * 8 + xor.leading_zeros() as u16);
            }
        }
        None
    }
}

impl<V: Clone> HotTree<V> {
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.root.is_null() {
            return None;
        }

        let mut current = self.root;

        // All nodes are binary (compaction is disabled)
        loop {
            if current.is_leaf() {
                let leaf_idx = current.leaf_index();
                let stored_key = self.get_leaf_key(leaf_idx);
                if stored_key == key {
                    let idx = self.get_leaf_value_idx(leaf_idx);
                    return self.values[idx].as_ref();
                }
                return None;
            }

            let (disc, left, right) = self.get_node(current.node_index());
            let bit = Self::bit_at(key, disc);
            current = if bit == 0 { left } else { right };
        }
    }

    pub fn contains_key(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        if self.root.is_null() {
            let leaf_idx = self.store_leaf(key);
            self.values.push(Some(value));
            self.root = Ptr::leaf(leaf_idx);
            self.count += 1;
            return None;
        }

        if self.root.is_leaf() {
            let leaf_idx = self.root.leaf_index();
            let existing_key = self.get_leaf_key(leaf_idx);

            if existing_key == key {
                let idx = self.get_leaf_value_idx(leaf_idx);
                let old = self.values[idx].take();
                self.values[idx] = Some(value);
                return old;
            }

            if let Some(diff_bit) = Self::first_diff_bit(&existing_key, key) {
                let new_leaf_idx = self.store_leaf(key);
                self.values.push(Some(value));
                let new_leaf = Ptr::leaf(new_leaf_idx);
                self.count += 1;

                let existing_bit = Self::bit_at(&existing_key, diff_bit);
                let (left, right) = if existing_bit == 0 {
                    (self.root, new_leaf)
                } else {
                    (new_leaf, self.root)
                };

                let node_idx = self.alloc_node(diff_bit, left, right);
                self.root = Ptr::node(node_idx);
            }
            return None;
        }

        self.insert_inner(key, value)
    }

    fn insert_inner(&mut self, key: &[u8], value: V) -> Option<V> {
        let mut path: Vec<(u32, bool)> = Vec::with_capacity(64);
        let mut current = self.root;

        loop {
            if current.is_leaf() {
                let leaf_idx = current.leaf_index();
                let existing_key = self.get_leaf_key(leaf_idx);

                if existing_key == key {
                    let idx = self.get_leaf_value_idx(leaf_idx);
                    let old = self.values[idx].take();
                    self.values[idx] = Some(value);
                    return old;
                }

                let diff_bit = Self::first_diff_bit(&existing_key, key)?;

                let new_leaf_idx = self.store_leaf(key);
                self.values.push(Some(value));
                let new_leaf = Ptr::leaf(new_leaf_idx);
                self.count += 1;

                let insert_point = self.find_insert_point(&path, diff_bit);

                // Verify invariant: diff_bit should not equal any discriminator on path
                #[cfg(debug_assertions)]
                for (node_idx, _) in &path {
                    let (disc, _, _) = self.get_node(*node_idx);
                    if disc == diff_bit {
                        panic!("INVARIANT VIOLATION: diff_bit {} equals node disc on path!", diff_bit);
                    }
                }

                if insert_point == path.len() {
                    // Normal case: new_node separates existing_leaf from new_leaf
                    let existing_bit = Self::bit_at(&existing_key, diff_bit);
                    let (left, right) = if existing_bit == 0 {
                        (current, new_leaf)
                    } else {
                        (new_leaf, current)
                    };

                    let new_node_idx = self.alloc_node(diff_bit, left, right);
                    let new_node = Ptr::node(new_node_idx);

                    if let Some(&(parent_idx, is_right)) = path.last() {
                        if is_right {
                            self.set_node_right(parent_idx, new_node);
                        } else {
                            self.set_node_left(parent_idx, new_node);
                        }
                    } else {
                        self.root = new_node;
                    }
                } else {
                    // Splice case: new_node separates new_leaf from subtree at insert_point
                    // (existing_leaf is somewhere inside that subtree)
                    let splice_node_idx = path[insert_point].0;
                    let splice_node_ptr = Ptr::node(splice_node_idx);

                    let new_key_bit = Self::bit_at(key, diff_bit);
                    let (left, right) = if new_key_bit == 0 {
                        (new_leaf, splice_node_ptr)
                    } else {
                        (splice_node_ptr, new_leaf)
                    };

                    let new_node_idx = self.alloc_node(diff_bit, left, right);
                    let new_node = Ptr::node(new_node_idx);

                    if insert_point == 0 {
                        self.root = new_node;
                    } else {
                        let (parent_idx, is_right) = path[insert_point - 1];
                        if is_right {
                            self.set_node_right(parent_idx, new_node);
                        } else {
                            self.set_node_left(parent_idx, new_node);
                        }
                    }
                }

                // Track max depth
                let new_depth = path.len() + 1;
                if new_depth > 500 && new_depth > self.max_depth_seen {
                    // Something went wrong - depth should be bounded by key_len * 8
                    eprintln!("DEPTH EXPLOSION: {} -> {}", self.max_depth_seen, new_depth);
                    eprintln!("  key len={}, diff_bit={}", key.len(), diff_bit);
                    eprintln!("  insert_point={}, path.len()={}", insert_point, path.len());
                    if insert_point < path.len() {
                        eprintln!("  SPLICE CASE");
                    } else {
                        eprintln!("  NORMAL CASE");
                    }
                    // Print path discriminators
                    let path_discs: Vec<u16> = path.iter()
                        .map(|(idx, _)| self.get_node(*idx).0)
                        .collect();
                    eprintln!("  path discs (first 20): {:?}", &path_discs[..path_discs.len().min(20)]);
                    eprintln!("  path discs (last 20): {:?}", &path_discs[path_discs.len().saturating_sub(20)..]);
                }
                self.max_depth_seen = self.max_depth_seen.max(new_depth);
                return None;
            }

            let node_idx = current.node_index();
            let (disc, left, right) = self.get_node(node_idx);
            let bit = Self::bit_at(key, disc);
            path.push((node_idx, bit == 1));
            current = if bit == 0 { left } else { right };
        }
    }

    fn find_insert_point(&self, path: &[(u32, bool)], new_disc: u16) -> usize {
        for (i, &(node_idx, _)) in path.iter().enumerate() {
            let (disc, _, _) = self.get_node(node_idx);
            if disc > new_disc {
                return i;
            }
        }
        path.len()
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<V> {
        if self.root.is_null() {
            return None;
        }

        let mut current = self.root;
        loop {
            if current.is_leaf() {
                let leaf_idx = current.leaf_index();
                let stored_key = self.get_leaf_key(leaf_idx);
                if stored_key == key {
                    let idx = self.get_leaf_value_idx(leaf_idx);
                    let old = self.values[idx].take();
                    if old.is_some() {
                        self.count -= 1;
                    }
                    return old;
                }
                return None;
            }

            let (disc, left, right) = self.get_node(current.node_index());
            let bit = Self::bit_at(key, disc);
            current = if bit == 0 { left } else { right };
        }
    }

    pub fn iter(&self) -> Iter<'_, V> {
        let mut stack = Vec::new();
        if !self.root.is_null() {
            stack.push(self.root);
        }
        Iter { tree: self, stack }
    }
}

impl<V> Default for HotTree<V> {
    fn default() -> Self { Self::new() }
}

impl<V: Clone> Clone for HotTree<V> {
    fn clone(&self) -> Self {
        Self {
            prefix_pool: self.prefix_pool.clone(),
            prefix_offsets: self.prefix_offsets.clone(),
            prefix_hash: self.prefix_hash.clone(),
            leaves: self.leaves.clone(),
            leaf_offsets: self.leaf_offsets.clone(),
            values: self.values.clone(),
            nodes: self.nodes.clone(),
            node_offsets: self.node_offsets.clone(),
            root: self.root,
            count: self.count,
            compacted: self.compacted,
            max_depth_seen: self.max_depth_seen,
            _marker: PhantomData,
        }
    }
}

impl<V: std::fmt::Debug + Clone> std::fmt::Debug for HotTree<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

pub struct Iter<'a, V> {
    tree: &'a HotTree<V>,
    stack: Vec<Ptr>,
}

impl<'a, V: Clone> Iterator for Iter<'a, V> {
    type Item = (Vec<u8>, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ptr) = self.stack.pop() {
            if ptr.is_null() {
                continue;
            }

            if ptr.is_leaf() {
                let leaf_idx = ptr.leaf_index();
                let idx = self.tree.get_leaf_value_idx(leaf_idx);
                if let Some(ref value) = self.tree.values[idx] {
                    let key = self.tree.get_leaf_key(leaf_idx);
                    return Some((key, value));
                }
                continue;
            }

            // All nodes are binary (compaction is disabled)
            let (_, left, right) = self.tree.get_node(ptr.node_index());
            self.stack.push(right);
            self.stack.push(left);
        }
        None
    }
}

#[cfg(test)]
mod proptests;

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
        assert_eq!(t.get(b"a"), Some(&1));
        assert_eq!(t.get(b"c"), Some(&3));
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
    fn test_prefix_compression() {
        let mut t: HotTree<u64> = HotTree::new();
        // URLs with shared prefix
        t.insert(b"https://example.com/page1", 1);
        t.insert(b"https://example.com/page2", 2);
        t.insert(b"https://example.com/page3", 3);
        t.insert(b"https://other.com/page1", 4);

        assert_eq!(t.get(b"https://example.com/page1"), Some(&1));
        assert_eq!(t.get(b"https://example.com/page2"), Some(&2));
        assert_eq!(t.get(b"https://example.com/page3"), Some(&3));
        assert_eq!(t.get(b"https://other.com/page1"), Some(&4));

        // Check that prefixes were learned
        assert!(t.prefix_offsets.len() > 1, "Should have learned prefixes");
    }

    #[test]
    fn test_iter() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"b", 2);
        t.insert(b"a", 1);
        t.insert(b"c", 3);

        let mut pairs: Vec<_> = t.iter().collect();
        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], (b"a".to_vec(), &1));
        assert_eq!(pairs[1], (b"b".to_vec(), &2));
        assert_eq!(pairs[2], (b"c".to_vec(), &3));
    }

    #[test]
    fn test_empty_key() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"", 42);
        assert_eq!(t.get(b""), Some(&42));
    }

    #[test]
    fn test_contains_key() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"exists", 1);
        assert!(t.contains_key(b"exists"));
        assert!(!t.contains_key(b"missing"));
    }

    #[test]
    fn test_clone() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        t.insert(b"b", 2);
        let t2 = t.clone();
        assert_eq!(t2.get(b"a"), Some(&1));
        assert_eq!(t2.get(b"b"), Some(&2));
    }

    #[test]
    fn test_compact() {
        let mut t: HotTree<u64> = HotTree::new();
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }

        // Verify before compaction
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            assert_eq!(t.get(key.as_bytes()), Some(&i), "Failed before compact at {}", i);
        }

        // Compact the tree
        let _ = t.compact();

        // Verify after compaction
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            assert_eq!(t.get(key.as_bytes()), Some(&i), "Failed after compact at {}", i);
        }

        // Test iterator after compaction
        let count = t.iter().count();
        assert_eq!(count, 100);
    }

    #[test]
    fn test_remove_then_reinsert_count() {
        use std::collections::BTreeMap;

        let mut tree: HotTree<u64> = HotTree::new();
        let mut model: BTreeMap<Vec<u8>, u64> = BTreeMap::new();

        let key = b"test_key";
        let key_vec = key.to_vec();

        // Insert key
        tree.insert(key, 1);
        model.insert(key_vec.clone(), 1);
        assert_eq!(tree.len(), 1);
        assert_eq!(model.len(), 1);

        // Remove key
        let tree_removed = tree.remove(key);
        let model_removed = model.remove(&key_vec);
        assert_eq!(tree_removed, Some(1));
        assert_eq!(model_removed, Some(1));
        assert_eq!(tree.len(), 0);
        assert_eq!(model.len(), 0);

        // Re-insert the same key
        let tree_insert_result = tree.insert(key, 2);
        let model_insert_result = model.insert(key_vec.clone(), 2);
        assert_eq!(tree_insert_result, None); // Should return None since key was removed
        assert_eq!(model_insert_result, None);

        // BUG: HotTree count doesn't increment when re-inserting a previously removed key.
        // The key still exists in the tree structure (with None value), so when insert()
        // finds the existing leaf and updates it, it doesn't increment the count.
        // This test documents the bug - it will pass once the bug is fixed.
        assert_eq!(
            tree.len(),
            model.len(),
            "BUG: Count mismatch after re-insert: HotTree={}, BTreeMap={}. \
             HotTree should increment count when re-inserting a previously removed key.",
            tree.len(),
            model.len()
        );
        assert_eq!(tree.get(key), model.get(&key_vec));
    }
}

