//! Optimized ART implementation based on DuckDB's design.
//!
//! Key optimizations:
//! - Smaller node structures with inline arrays
//! - Separate prefix chain for long prefixes
//! - Combined leaf+node types to reduce node count
//! - Arena-based allocation to reduce overhead

use std::ops::RangeBounds;

/// A 4-byte node reference (index into node arena).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct NodeIdx(u32);

impl NodeIdx {
    pub const NULL: NodeIdx = NodeIdx(u32::MAX);
    
    #[inline]
    pub fn is_null(self) -> bool {
        self.0 == u32::MAX
    }
    
    #[inline]
    fn new(idx: usize) -> Self {
        debug_assert!(idx < u32::MAX as usize);
        NodeIdx(idx as u32)
    }
    
    #[inline]
    fn idx(self) -> usize {
        self.0 as usize
    }
}

/// 4-byte data reference (offset in key arena).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(C, packed)]
pub struct KeyIdx {
    offset: u32,
}

impl KeyIdx {
    pub const EMPTY: KeyIdx = KeyIdx { offset: u32::MAX };
    
    #[inline]
    fn new(offset: usize) -> Self {
        debug_assert!(offset < u32::MAX as usize);
        Self { offset: offset as u32 }
    }
    
    #[inline]
    fn offset(self) -> usize {
        self.offset as usize
    }
    
    #[inline]
    fn is_empty(self) -> bool {
        self.offset == u32::MAX
    }
}

/// Optimized node structure.
/// Uses tagged union approach similar to DuckDB.
/// Node types encoded in the high bits of the first byte.
#[repr(C)]
pub struct OptNode<V: Clone + Copy> {
    /// Node type + inline prefix length (up to 7 bytes inline).
    /// Bits 7-5: node type (0=leaf, 1=node4, 2=node16, 3=node48, 4=node256)
    /// Bits 4-0: inline prefix length (0-15)
    header: u8,
    /// Inline prefix bytes (up to 15 bytes).
    prefix: [u8; 15],
    /// Node data (union based on type).
    data: NodeData<V>,
}

/// Node data union (40 bytes).
#[repr(C)]
union NodeData<V: Clone + Copy> {
    leaf: LeafData<V>,
    node4: Node4Data,
    node16: Node16Data,
    node48: Node48Data,
    node256: Node256Idx,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct LeafData<V: Clone + Copy> {
    key_idx: KeyIdx,
    value: V,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Node4Data {
    num_children: u8,
    keys: [u8; 4],
    children: [NodeIdx; 4],
    /// Key index if this node also stores a value (key ends here).
    leaf_key: KeyIdx,
    leaf_value_idx: u32, // Index into value arena
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Node16Data {
    num_children: u8,
    keys: [u8; 16],
    children: [NodeIdx; 16],
    leaf_key: KeyIdx,
    leaf_value_idx: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Node48Data {
    num_children: u8,
    child_idx: [u8; 48], // Maps byte -> child index (48 slots only - need separate lookup)
    // Actually need different approach for Node48...
}

#[derive(Clone, Copy)]
#[repr(C)]
struct Node256Idx {
    children_offset: u32, // Offset into a separate Node256 children array
    leaf_key: KeyIdx,
    leaf_value_idx: u32,
}

// The above approach is getting complex. Let me try a simpler but still efficient approach.

impl<V: Clone + Copy + Default> OptNode<V> {
    const TYPE_LEAF: u8 = 0 << 5;
    const TYPE_NODE4: u8 = 1 << 5;
    const TYPE_NODE16: u8 = 2 << 5;
    const TYPE_NODE48: u8 = 3 << 5;
    const TYPE_NODE256: u8 = 4 << 5;
    const TYPE_MASK: u8 = 0b11100000;
    const PREFIX_MASK: u8 = 0b00011111;
    
    fn node_type(&self) -> u8 {
        self.header & Self::TYPE_MASK
    }
    
    fn prefix_len(&self) -> usize {
        (self.header & Self::PREFIX_MASK) as usize
    }
    
    fn get_prefix(&self) -> &[u8] {
        &self.prefix[..self.prefix_len()]
    }
    
    fn new_leaf(prefix: &[u8], key_idx: KeyIdx, value: V) -> Self {
        let mut node = Self {
            header: Self::TYPE_LEAF | (prefix.len().min(15) as u8),
            prefix: [0; 15],
            data: NodeData { leaf: LeafData { key_idx, value } },
        };
        let copy_len = prefix.len().min(15);
        node.prefix[..copy_len].copy_from_slice(&prefix[..copy_len]);
        node
    }
    
    fn new_node4(prefix: &[u8]) -> Self {
        let mut node = Self {
            header: Self::TYPE_NODE4 | (prefix.len().min(15) as u8),
            prefix: [0; 15],
            data: NodeData {
                node4: Node4Data {
                    num_children: 0,
                    keys: [0; 4],
                    children: [NodeIdx::NULL; 4],
                    leaf_key: KeyIdx::EMPTY,
                    leaf_value_idx: u32::MAX,
                }
            },
        };
        let copy_len = prefix.len().min(15);
        node.prefix[..copy_len].copy_from_slice(&prefix[..copy_len]);
        node
    }
    
    fn is_leaf(&self) -> bool {
        self.node_type() == Self::TYPE_LEAF
    }
}

// This approach is getting too complex for a single file.
// Let me simplify and focus on what really matters: reducing node count.

/// Statistics for memory analysis.
#[derive(Debug, Clone, Default)]
pub struct OptArtStats {
    pub key_arena_bytes: usize,
    pub node_count: usize,
    pub node_arena_bytes: usize,
}

/// Simplified but optimized ART.
/// Key insight: The main overhead is too many nodes.
/// Solution: Aggressive path compression + combined leaf/node.
pub struct OptimizedArt<V: Clone> {
    /// Key data arena.
    keys: Vec<u8>,
    /// Nodes stored contiguously.
    nodes: Vec<CompactNode<V>>,
    /// Root node.
    root: NodeIdx,
    /// Number of keys.
    size: usize,
}

/// Compact node structure (48 bytes).
#[derive(Clone)]
pub enum CompactNode<V: Clone> {
    /// Leaf with full key stored.
    Leaf {
        /// Offset into key arena.
        key_offset: u32,
        /// Key length.
        key_len: u16,
        /// Value.
        value: V,
    },
    
    /// Node4 with inline prefix.
    Node4 {
        /// Prefix stored inline (up to 12 bytes).
        prefix: [u8; 12],
        prefix_len: u8,
        num_children: u8,
        keys: [u8; 4],
        children: [NodeIdx; 4],
        /// Optional leaf value for keys ending at this node.
        leaf: Option<(u32, u16, V)>, // (key_offset, key_len, value)
    },
    
    /// Node16 with separate prefix.
    Node16 {
        prefix_offset: u32,
        prefix_len: u16,
        num_children: u8,
        keys: [u8; 16],
        children: [NodeIdx; 16],
        leaf: Option<(u32, u16, V)>,
    },
    
    /// Node48.
    Node48 {
        prefix_offset: u32,
        prefix_len: u16,
        num_children: u8,
        child_idx: Box<[u8; 256]>,
        children: [NodeIdx; 48],
        leaf: Option<(u32, u16, V)>,
    },
    
    /// Node256.
    Node256 {
        prefix_offset: u32,
        prefix_len: u16,
        num_children: u16,
        children: Box<[NodeIdx; 256]>,
        leaf: Option<(u32, u16, V)>,
    },
}

impl<V: Clone> Default for CompactNode<V> {
    fn default() -> Self {
        CompactNode::Node4 {
            prefix: [0; 12],
            prefix_len: 0,
            num_children: 0,
            keys: [0; 4],
            children: [NodeIdx::NULL; 4],
            leaf: None,
        }
    }
}

impl<V: Clone> CompactNode<V> {
    fn is_leaf(&self) -> bool {
        matches!(self, CompactNode::Leaf { .. })
    }
    
    fn find_child(&self, byte: u8) -> Option<NodeIdx> {
        match self {
            CompactNode::Leaf { .. } => None,
            CompactNode::Node4 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        return Some(children[i]);
                    }
                }
                None
            }
            CompactNode::Node16 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        return Some(children[i]);
                    }
                }
                None
            }
            CompactNode::Node48 { child_idx, children, .. } => {
                let idx = child_idx[byte as usize];
                if idx != 255 {
                    Some(children[idx as usize])
                } else {
                    None
                }
            }
            CompactNode::Node256 { children, .. } => {
                let child = children[byte as usize];
                if !child.is_null() {
                    Some(child)
                } else {
                    None
                }
            }
        }
    }
    
    fn num_children(&self) -> usize {
        match self {
            CompactNode::Leaf { .. } => 0,
            CompactNode::Node4 { num_children, .. } => *num_children as usize,
            CompactNode::Node16 { num_children, .. } => *num_children as usize,
            CompactNode::Node48 { num_children, .. } => *num_children as usize,
            CompactNode::Node256 { num_children, .. } => *num_children as usize,
        }
    }
    
    fn should_grow(&self) -> bool {
        match self {
            CompactNode::Leaf { .. } => false,
            CompactNode::Node4 { num_children, .. } => *num_children >= 4,
            CompactNode::Node16 { num_children, .. } => *num_children >= 16,
            CompactNode::Node48 { num_children, .. } => *num_children >= 48,
            CompactNode::Node256 { .. } => false,
        }
    }
    
    fn set_child(&mut self, byte: u8, child: NodeIdx) {
        match self {
            CompactNode::Leaf { .. } => panic!("Cannot set child on leaf"),
            CompactNode::Node4 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        children[i] = child;
                        return;
                    }
                }
                assert!((*num_children as usize) < 4);
                let idx = *num_children as usize;
                keys[idx] = byte;
                children[idx] = child;
                *num_children += 1;
            }
            CompactNode::Node16 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        children[i] = child;
                        return;
                    }
                }
                assert!((*num_children as usize) < 16);
                let idx = *num_children as usize;
                keys[idx] = byte;
                children[idx] = child;
                *num_children += 1;
            }
            CompactNode::Node48 { child_idx, num_children, children, .. } => {
                let existing = child_idx[byte as usize];
                if existing != 255 {
                    children[existing as usize] = child;
                } else {
                    assert!((*num_children as usize) < 48);
                    let slot = *num_children as usize;
                    children[slot] = child;
                    child_idx[byte as usize] = slot as u8;
                    *num_children += 1;
                }
            }
            CompactNode::Node256 { children, num_children, .. } => {
                if children[byte as usize].is_null() {
                    *num_children += 1;
                }
                children[byte as usize] = child;
            }
        }
    }
}

impl<V: Clone> OptimizedArt<V> {
    pub fn new() -> Self {
        Self {
            keys: Vec::with_capacity(64 * 1024),
            nodes: Vec::with_capacity(1024),
            root: NodeIdx::NULL,
            size: 0,
        }
    }
    
    fn store_key(&mut self, key: &[u8]) -> (u32, u16) {
        let offset = self.keys.len() as u32;
        self.keys.extend_from_slice(key);
        (offset, key.len() as u16)
    }
    
    fn get_key(&self, offset: u32, len: u16) -> &[u8] {
        &self.keys[offset as usize..(offset as usize + len as usize)]
    }
    
    fn alloc_node(&mut self, node: CompactNode<V>) -> NodeIdx {
        let idx = self.nodes.len();
        self.nodes.push(node);
        NodeIdx::new(idx)
    }
    
    fn node(&self, idx: NodeIdx) -> &CompactNode<V> {
        &self.nodes[idx.idx()]
    }
    
    fn node_mut(&mut self, idx: NodeIdx) -> &mut CompactNode<V> {
        &mut self.nodes[idx.idx()]
    }
    
    /// Insert a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        let (key_offset, key_len) = self.store_key(key);
        
        if self.root.is_null() {
            let leaf = CompactNode::Leaf { 
                key_offset, 
                key_len, 
                value 
            };
            self.root = self.alloc_node(leaf);
            self.size = 1;
            return None;
        }
        
        let result = self.insert_impl(key, key_offset, key_len, value);
        if result.is_none() {
            self.size += 1;
        }
        result
    }
    
    fn insert_impl(&mut self, key: &[u8], key_offset: u32, key_len: u16, value: V) -> Option<V> {
        let mut path: Vec<(NodeIdx, u8)> = Vec::with_capacity(key.len() / 4);
        let mut current = self.root;
        let mut depth = 0;
        
        loop {
            // Get node info without holding borrow
            let (is_leaf, node_key, num_children) = {
                let node = self.node(current);
                match node {
                    CompactNode::Leaf { key_offset: ko, key_len: kl, .. } => {
                        (true, Some((*ko, *kl)), 0)
                    }
                    _ => (false, None, node.num_children())
                }
            };
            
            if is_leaf {
                let (ko, kl) = node_key.unwrap();
                let existing_key = self.get_key(ko, kl).to_vec();
                
                if existing_key == key {
                    // Same key - replace value
                    if let CompactNode::Leaf { value: v, .. } = self.node_mut(current) {
                        return Some(std::mem::replace(v, value));
                    }
                }
                
                // Split the leaf
                let common = existing_key[depth..]
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                let split_depth = depth + common;
                let existing_byte = existing_key.get(split_depth).copied();
                let new_byte = key.get(split_depth).copied();
                
                // Create new node
                let prefix = &key[depth..split_depth];
                let mut new_node = if prefix.len() <= 12 {
                    let mut p = [0u8; 12];
                    p[..prefix.len()].copy_from_slice(prefix);
                    CompactNode::Node4 {
                        prefix: p,
                        prefix_len: prefix.len() as u8,
                        num_children: 0,
                        keys: [0; 4],
                        children: [NodeIdx::NULL; 4],
                        leaf: None,
                    }
                } else {
                    let (po, pl) = self.store_key(prefix);
                    CompactNode::Node16 {
                        prefix_offset: po,
                        prefix_len: pl,
                        num_children: 0,
                        keys: [0; 16],
                        children: [NodeIdx::NULL; 16],
                        leaf: None,
                    }
                };
                
                match (existing_byte, new_byte) {
                    (Some(eb), Some(nb)) => {
                        let new_leaf = CompactNode::Leaf { key_offset, key_len, value };
                        let new_leaf_idx = self.alloc_node(new_leaf);
                        new_node.set_child(eb, current);
                        new_node.set_child(nb, new_leaf_idx);
                    }
                    (Some(eb), None) => {
                        new_node.set_child(eb, current);
                        // Key ends here - store as leaf in node
                        match &mut new_node {
                            CompactNode::Node4 { leaf, .. } | 
                            CompactNode::Node16 { leaf, .. } => {
                                *leaf = Some((key_offset, key_len, value));
                            }
                            _ => unreachable!()
                        }
                    }
                    (None, Some(nb)) => {
                        // Existing key ends at this node
                        if let CompactNode::Leaf { key_offset: ko, key_len: kl, value: v } 
                            = self.node(current).clone() 
                        {
                            match &mut new_node {
                                CompactNode::Node4 { leaf, .. } |
                                CompactNode::Node16 { leaf, .. } => {
                                    *leaf = Some((ko, kl, v));
                                }
                                _ => unreachable!()
                            }
                        }
                        let new_leaf = CompactNode::Leaf { key_offset, key_len, value };
                        let new_leaf_idx = self.alloc_node(new_leaf);
                        new_node.set_child(nb, new_leaf_idx);
                    }
                    (None, None) => unreachable!()
                }
                
                let new_node_idx = self.alloc_node(new_node);
                
                if path.is_empty() {
                    self.root = new_node_idx;
                } else {
                    let (parent, byte) = path.last().unwrap();
                    self.node_mut(*parent).set_child(*byte, new_node_idx);
                }
                
                return None;
            }
            
            // Internal node - check prefix
            let (prefix, prefix_matches) = {
                let node = self.node(current);
                let prefix = match node {
                    CompactNode::Node4 { prefix, prefix_len, .. } => {
                        prefix[..*prefix_len as usize].to_vec()
                    }
                    CompactNode::Node16 { prefix_offset, prefix_len, .. } |
                    CompactNode::Node48 { prefix_offset, prefix_len, .. } |
                    CompactNode::Node256 { prefix_offset, prefix_len, .. } => {
                        self.get_key(*prefix_offset, *prefix_len).to_vec()
                    }
                    CompactNode::Leaf { .. } => unreachable!()
                };
                
                let key_remaining = &key[depth..];
                let matches = key_remaining.len() >= prefix.len() && 
                    &key_remaining[..prefix.len()] == prefix.as_slice();
                (prefix, matches)
            };
            
            if !prefix_matches {
                // Prefix mismatch - need to split
                let key_remaining = &key[depth..];
                let mismatch = key_remaining
                    .iter()
                    .zip(prefix.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                // Create split node
                let split_prefix = &prefix[..mismatch];
                let mut split_node = if split_prefix.len() <= 12 {
                    let mut p = [0u8; 12];
                    p[..split_prefix.len()].copy_from_slice(split_prefix);
                    CompactNode::Node4 {
                        prefix: p,
                        prefix_len: split_prefix.len() as u8,
                        num_children: 0,
                        keys: [0; 4],
                        children: [NodeIdx::NULL; 4],
                        leaf: None,
                    }
                } else {
                    let (po, pl) = self.store_key(split_prefix);
                    CompactNode::Node16 {
                        prefix_offset: po,
                        prefix_len: pl,
                        num_children: 0,
                        keys: [0; 16],
                        children: [NodeIdx::NULL; 16],
                        leaf: None,
                    }
                };
                
                // Update current node's prefix
                let remaining_prefix = prefix[mismatch + 1..].to_vec();
                let existing_byte = prefix[mismatch];
                
                // Store remaining prefix first to avoid borrow issues
                let (new_po, new_pl) = self.store_key(&remaining_prefix);
                
                match self.node_mut(current) {
                    CompactNode::Node4 { prefix: p, prefix_len, .. } => {
                        let copy_len = remaining_prefix.len().min(12);
                        p[..copy_len].copy_from_slice(&remaining_prefix[..copy_len]);
                        *prefix_len = copy_len as u8;
                    }
                    CompactNode::Node16 { prefix_offset, prefix_len, .. } |
                    CompactNode::Node48 { prefix_offset, prefix_len, .. } |
                    CompactNode::Node256 { prefix_offset, prefix_len, .. } => {
                        *prefix_offset = new_po;
                        *prefix_len = new_pl;
                    }
                    _ => unreachable!()
                }
                
                split_node.set_child(existing_byte, current);
                
                // Handle new key
                if depth + mismatch >= key.len() {
                    match &mut split_node {
                        CompactNode::Node4 { leaf, .. } |
                        CompactNode::Node16 { leaf, .. } => {
                            *leaf = Some((key_offset, key_len, value));
                        }
                        _ => unreachable!()
                    }
                } else {
                    let new_byte = key[depth + mismatch];
                    let new_leaf = CompactNode::Leaf { key_offset, key_len, value };
                    let new_leaf_idx = self.alloc_node(new_leaf);
                    split_node.set_child(new_byte, new_leaf_idx);
                }
                
                let split_idx = self.alloc_node(split_node);
                
                if path.is_empty() {
                    self.root = split_idx;
                } else {
                    let (parent, byte) = path.last().unwrap();
                    self.node_mut(*parent).set_child(*byte, split_idx);
                }
                
                return None;
            }
            
            depth += prefix.len();
            
            // Check if key ends here
            if depth >= key.len() {
                let old = match self.node_mut(current) {
                    CompactNode::Node4 { leaf, .. } |
                    CompactNode::Node16 { leaf, .. } |
                    CompactNode::Node48 { leaf, .. } |
                    CompactNode::Node256 { leaf, .. } => {
                        let old = leaf.take().map(|(_, _, v)| v);
                        *leaf = Some((key_offset, key_len, value));
                        old
                    }
                    _ => None
                };
                return old;
            }
            
            // Find child
            let next_byte = key[depth];
            let child = self.node(current).find_child(next_byte);
            
            if let Some(c) = child {
                path.push((current, next_byte));
                current = c;
                depth += 1;
            } else {
                // Add new leaf
                let new_leaf = CompactNode::Leaf { key_offset, key_len, value };
                let new_leaf_idx = self.alloc_node(new_leaf);
                
                // Grow if needed
                if self.node(current).should_grow() {
                    self.grow_node(current);
                }
                
                self.node_mut(current).set_child(next_byte, new_leaf_idx);
                return None;
            }
        }
    }
    
    fn grow_node(&mut self, idx: NodeIdx) {
        let old = std::mem::take(self.node_mut(idx));
        let new = match old {
            CompactNode::Node4 { prefix, prefix_len, num_children, keys, children, leaf } => {
                let (po, pl) = self.store_key(&prefix[..prefix_len as usize]);
                let mut new_keys = [0u8; 16];
                let mut new_children = [NodeIdx::NULL; 16];
                for i in 0..num_children as usize {
                    new_keys[i] = keys[i];
                    new_children[i] = children[i];
                }
                CompactNode::Node16 {
                    prefix_offset: po,
                    prefix_len: pl,
                    num_children,
                    keys: new_keys,
                    children: new_children,
                    leaf,
                }
            }
            CompactNode::Node16 { prefix_offset, prefix_len, num_children, keys, children, leaf } => {
                let mut child_idx = Box::new([255u8; 256]);
                let mut new_children = [NodeIdx::NULL; 48];
                for i in 0..num_children as usize {
                    child_idx[keys[i] as usize] = i as u8;
                    new_children[i] = children[i];
                }
                CompactNode::Node48 {
                    prefix_offset,
                    prefix_len,
                    num_children,
                    child_idx,
                    children: new_children,
                    leaf,
                }
            }
            CompactNode::Node48 { prefix_offset, prefix_len, num_children, child_idx, children, leaf } => {
                let mut new_children = Box::new([NodeIdx::NULL; 256]);
                for byte in 0..256 {
                    let idx = child_idx[byte];
                    if idx != 255 {
                        new_children[byte] = children[idx as usize];
                    }
                }
                CompactNode::Node256 {
                    prefix_offset,
                    prefix_len,
                    num_children: num_children as u16,
                    children: new_children,
                    leaf,
                }
            }
            other => other
        };
        *self.node_mut(idx) = new;
    }
    
    /// Get a value by key.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.root.is_null() {
            return None;
        }
        
        let mut current = self.root;
        let mut depth = 0;
        
        loop {
            let node = self.node(current);
            
            match node {
                CompactNode::Leaf { key_offset, key_len, value } => {
                    let stored = self.get_key(*key_offset, *key_len);
                    if stored == key {
                        return Some(value);
                    }
                    return None;
                }
                _ => {
                    // Get prefix
                    let prefix = match node {
                        CompactNode::Node4 { prefix, prefix_len, .. } => {
                            &prefix[..*prefix_len as usize]
                        }
                        CompactNode::Node16 { prefix_offset, prefix_len, .. } |
                        CompactNode::Node48 { prefix_offset, prefix_len, .. } |
                        CompactNode::Node256 { prefix_offset, prefix_len, .. } => {
                            self.get_key(*prefix_offset, *prefix_len)
                        }
                        _ => unreachable!()
                    };
                    
                    // Check prefix
                    if key.len() < depth + prefix.len() || &key[depth..depth + prefix.len()] != prefix {
                        return None;
                    }
                    depth += prefix.len();
                    
                    // Check if key ends here
                    if depth >= key.len() {
                        let leaf = match node {
                            CompactNode::Node4 { leaf, .. } |
                            CompactNode::Node16 { leaf, .. } |
                            CompactNode::Node48 { leaf, .. } |
                            CompactNode::Node256 { leaf, .. } => leaf,
                            _ => return None,
                        };
                        if let Some((ko, kl, v)) = leaf {
                            if self.get_key(*ko, *kl) == key {
                                return Some(v);
                            }
                        }
                        return None;
                    }
                    
                    // Find child
                    let next_byte = key[depth];
                    if let Some(child) = node.find_child(next_byte) {
                        current = child;
                        depth += 1;
                    } else {
                        return None;
                    }
                }
            }
        }
    }
    
    pub fn len(&self) -> usize {
        self.size
    }
    
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    
    pub fn memory_stats(&self) -> OptArtStats {
        OptArtStats {
            key_arena_bytes: self.keys.capacity(),
            node_count: self.nodes.len(),
            node_arena_bytes: self.nodes.capacity() * std::mem::size_of::<CompactNode<V>>(),
        }
    }
}

impl<V: Clone> Default for OptimizedArt<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut tree: OptimizedArt<u64> = OptimizedArt::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        tree.insert(b"help", 3);
        tree.insert(b"he", 4);
        
        assert_eq!(tree.get(b"hello"), Some(&1));
        assert_eq!(tree.get(b"world"), Some(&2));
        assert_eq!(tree.get(b"help"), Some(&3));
        assert_eq!(tree.get(b"he"), Some(&4));
        assert_eq!(tree.get(b"hel"), None);
    }
    
    #[test]
    fn test_many() {
        let mut tree: OptimizedArt<u64> = OptimizedArt::new();
        
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            assert_eq!(tree.get(key.as_bytes()), Some(&i));
        }
    }
    
    #[test]
    fn test_node_size() {
        println!("CompactNode<u64>: {} bytes", std::mem::size_of::<CompactNode<u64>>());
    }
}
