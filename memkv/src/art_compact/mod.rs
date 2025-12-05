//! Compact Adaptive Radix Tree (ART) with arena-based key storage.
//!
//! This variant uses arena allocation for keys to reduce memory overhead:
//! - Keys stored in a contiguous arena with 6-byte references (offset + length)
//! - Instead of 24-byte `Vec<u8>` per key
//! - Better cache locality for key data

use std::ops::{Bound, RangeBounds};

/// A 32-bit offset into the key arena.
/// Uses packed representation to save space (6 bytes instead of 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, packed)]
pub struct KeyRef {
    offset: u32,
    len: u16,
}

impl KeyRef {
    pub const fn null() -> Self {
        Self { offset: u32::MAX, len: 0 }
    }
    
    pub fn is_null(&self) -> bool {
        self.offset == u32::MAX
    }
    
    pub fn new(offset: usize, len: usize) -> Self {
        debug_assert!(offset < u32::MAX as usize);
        debug_assert!(len < u16::MAX as usize);
        Self { 
            offset: offset as u32, 
            len: len as u16 
        }
    }
    
    pub fn offset(&self) -> usize {
        self.offset as usize
    }
    
    pub fn len(&self) -> usize {
        self.len as usize
    }
    
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Arena for storing keys contiguously.
pub struct KeyArena {
    data: Vec<u8>,
}

impl KeyArena {
    pub fn new() -> Self {
        Self::with_capacity(64 * 1024) // 64KB initial
    }
    
    pub fn with_capacity(cap: usize) -> Self {
        Self { data: Vec::with_capacity(cap) }
    }
    
    /// Store a key and return its reference.
    pub fn store(&mut self, key: &[u8]) -> KeyRef {
        if key.is_empty() {
            return KeyRef { offset: 0, len: 0 };
        }
        let offset = self.data.len();
        self.data.extend_from_slice(key);
        KeyRef::new(offset, key.len())
    }
    
    /// Get a key by reference.
    pub fn get(&self, key_ref: KeyRef) -> &[u8] {
        if key_ref.is_null() || key_ref.len() == 0 {
            return &[];
        }
        let start = key_ref.offset();
        let end = start + key_ref.len();
        &self.data[start..end]
    }
    
    /// Total bytes used.
    pub fn len(&self) -> usize {
        self.data.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    
    /// Total capacity.
    pub fn capacity(&self) -> usize {
        self.data.capacity()
    }
}

impl Default for KeyArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory statistics for the compact ART.
#[derive(Debug, Clone, Default)]
pub struct CompactArtStats {
    /// Bytes used in key arena
    pub key_arena_bytes: usize,
    /// Bytes used for node structures  
    pub node_bytes: usize,
    /// Number of Node4 instances
    pub node4_count: usize,
    /// Number of Node16 instances
    pub node16_count: usize,
    /// Number of Node48 instances
    pub node48_count: usize,
    /// Number of Node256 instances
    pub node256_count: usize,
    /// Number of leaf nodes
    pub leaf_count: usize,
    /// Number of internal nodes with leaf values
    pub internal_leaf_count: usize,
}

/// A node in the Compact ART.
pub enum CompactNode<V> {
    /// A leaf node with arena-backed key reference.
    Leaf {
        key_ref: KeyRef,
        value: V,
    },
    
    /// A node with up to 4 children.
    Node4 {
        prefix: Vec<u8>,
        num_children: u8,
        keys: [u8; 4],
        children: [Option<Box<CompactNode<V>>>; 4],
        /// Leaf value if a key ends at this node.
        leaf_value: Option<(KeyRef, V)>,
    },
    
    /// A node with 5-16 children.
    Node16 {
        prefix: Vec<u8>,
        num_children: u8,
        keys: [u8; 16],
        children: [Option<Box<CompactNode<V>>>; 16],
        leaf_value: Option<(KeyRef, V)>,
    },
    
    /// A node with 17-48 children.
    Node48 {
        prefix: Vec<u8>,
        num_children: u8,
        child_index: Box<[u8; 256]>,
        children: Vec<Option<Box<CompactNode<V>>>>,
        leaf_value: Option<(KeyRef, V)>,
    },
    
    /// A node with 49-256 children.
    Node256 {
        prefix: Vec<u8>,
        num_children: u16,
        children: Box<[Option<Box<CompactNode<V>>>; 256]>,
        leaf_value: Option<(KeyRef, V)>,
    },
}

impl<V> CompactNode<V> {
    pub fn new_leaf(key_ref: KeyRef, value: V) -> Self {
        CompactNode::Leaf { key_ref, value }
    }
    
    pub fn new_node4() -> Self {
        CompactNode::Node4 {
            prefix: Vec::new(),
            num_children: 0,
            keys: [0; 4],
            children: [None, None, None, None],
            leaf_value: None,
        }
    }
    
    pub fn new_node16() -> Self {
        CompactNode::Node16 {
            prefix: Vec::new(),
            num_children: 0,
            keys: [0; 16],
            children: Default::default(),
            leaf_value: None,
        }
    }
    
    pub fn new_node48() -> Self {
        CompactNode::Node48 {
            prefix: Vec::new(),
            num_children: 0,
            child_index: Box::new([255; 256]),
            children: Vec::with_capacity(48),
            leaf_value: None,
        }
    }
    
    pub fn new_node256() -> Self {
        CompactNode::Node256 {
            prefix: Vec::new(),
            num_children: 0,
            children: Box::new(std::array::from_fn(|_| None)),
            leaf_value: None,
        }
    }
    
    pub fn num_children(&self) -> usize {
        match self {
            CompactNode::Leaf { .. } => 0,
            CompactNode::Node4 { num_children, .. } => *num_children as usize,
            CompactNode::Node16 { num_children, .. } => *num_children as usize,
            CompactNode::Node48 { num_children, .. } => *num_children as usize,
            CompactNode::Node256 { num_children, .. } => *num_children as usize,
        }
    }
    
    pub fn prefix(&self) -> &[u8] {
        match self {
            CompactNode::Leaf { .. } => &[],
            CompactNode::Node4 { prefix, .. }
            | CompactNode::Node16 { prefix, .. }
            | CompactNode::Node48 { prefix, .. }
            | CompactNode::Node256 { prefix, .. } => prefix,
        }
    }
    
    pub fn set_prefix(&mut self, new_prefix: &[u8]) {
        match self {
            CompactNode::Leaf { .. } => {}
            CompactNode::Node4 { prefix, .. }
            | CompactNode::Node16 { prefix, .. }
            | CompactNode::Node48 { prefix, .. }
            | CompactNode::Node256 { prefix, .. } => {
                prefix.clear();
                prefix.extend_from_slice(new_prefix);
            }
        }
    }
    
    pub fn find_child(&self, key: u8) -> Option<usize> {
        match self {
            CompactNode::Leaf { .. } => None,
            CompactNode::Node4 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(i);
                    }
                }
                None
            }
            CompactNode::Node16 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(i);
                    }
                }
                None
            }
            CompactNode::Node48 { child_index, .. } => {
                let idx = child_index[key as usize];
                if idx != 255 {
                    Some(idx as usize)
                } else {
                    None
                }
            }
            CompactNode::Node256 { children, .. } => {
                if children[key as usize].is_some() {
                    Some(key as usize)
                } else {
                    None
                }
            }
        }
    }
    
    pub fn add_child(&mut self, key: u8, child: Box<CompactNode<V>>) {
        match self {
            CompactNode::Leaf { .. } => panic!("Cannot add child to leaf"),
            CompactNode::Node4 { keys, num_children, children, .. } => {
                // Check if key already exists
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = Some(child);
                        return;
                    }
                }
                
                if (*num_children as usize) < 4 {
                    let idx = *num_children as usize;
                    keys[idx] = key;
                    children[idx] = Some(child);
                    *num_children += 1;
                } else {
                    panic!("Node4 is full");
                }
            }
            CompactNode::Node16 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = Some(child);
                        return;
                    }
                }
                
                if (*num_children as usize) < 16 {
                    let idx = *num_children as usize;
                    keys[idx] = key;
                    children[idx] = Some(child);
                    *num_children += 1;
                } else {
                    panic!("Node16 is full");
                }
            }
            CompactNode::Node48 { child_index, num_children, children, .. } => {
                let existing_idx = child_index[key as usize];
                if existing_idx != 255 && (existing_idx as usize) < children.len() {
                    children[existing_idx as usize] = Some(child);
                } else if (*num_children as usize) < 48 {
                    // Find free slot
                    let slot = children.iter().position(|c| c.is_none())
                        .unwrap_or(children.len());
                    if slot < children.len() {
                        children[slot] = Some(child);
                    } else {
                        children.push(Some(child));
                    }
                    child_index[key as usize] = slot as u8;
                    *num_children += 1;
                } else {
                    panic!("Node48 is full");
                }
            }
            CompactNode::Node256 { children, num_children, .. } => {
                if children[key as usize].is_none() {
                    *num_children += 1;
                }
                children[key as usize] = Some(child);
            }
        }
    }
    
    pub fn remove_child(&mut self, idx: usize) -> Box<CompactNode<V>> {
        match self {
            CompactNode::Leaf { .. } => panic!("Cannot remove from leaf"),
            CompactNode::Node4 { keys, num_children, children, .. } => {
                let child = children[idx].take().expect("Child should exist");
                // Compact
                for i in idx..(*num_children as usize - 1) {
                    keys[i] = keys[i + 1];
                    children.swap(i, i + 1);
                }
                *num_children -= 1;
                child
            }
            CompactNode::Node16 { keys, num_children, children, .. } => {
                let child = children[idx].take().expect("Child should exist");
                for i in idx..(*num_children as usize - 1) {
                    keys[i] = keys[i + 1];
                    children.swap(i, i + 1);
                }
                *num_children -= 1;
                child
            }
            CompactNode::Node48 { child_index, num_children, children, .. } => {
                // Find the key that maps to this index
                let mut key_byte = None;
                for (k, &i) in child_index.iter().enumerate() {
                    if i != 255 && i as usize == idx {
                        key_byte = Some(k);
                        break;
                    }
                }
                
                let child = children[idx].take().expect("Child should exist");
                
                if let Some(kb) = key_byte {
                    child_index[kb] = 255;
                    *num_children -= 1;
                }
                
                child
            }
            CompactNode::Node256 { children, num_children, .. } => {
                *num_children -= 1;
                children[idx].take().expect("Child should exist")
            }
        }
    }
    
    pub fn should_grow(&self) -> bool {
        match self {
            CompactNode::Leaf { .. } => false,
            CompactNode::Node4 { num_children, .. } => *num_children >= 4,
            CompactNode::Node16 { num_children, .. } => *num_children >= 16,
            CompactNode::Node48 { num_children, .. } => *num_children >= 48,
            CompactNode::Node256 { .. } => false,
        }
    }
    
    pub fn grow(&mut self, stats: &mut CompactArtStats) {
        match self {
            CompactNode::Node4 { prefix, keys, num_children, children, leaf_value } => {
                let mut new_keys = [0u8; 16];
                new_keys[..4].copy_from_slice(keys);
                
                let mut new_children: [Option<Box<CompactNode<V>>>; 16] = Default::default();
                for i in 0..4 {
                    new_children[i] = children[i].take();
                }
                
                *self = CompactNode::Node16 {
                    prefix: std::mem::take(prefix),
                    num_children: *num_children,
                    keys: new_keys,
                    children: new_children,
                    leaf_value: std::mem::take(leaf_value),
                };
                
                stats.node4_count = stats.node4_count.saturating_sub(1);
                stats.node16_count += 1;
            }
            CompactNode::Node16 { prefix, keys, num_children, children, leaf_value } => {
                let mut child_index = Box::new([255u8; 256]);
                let mut new_children = Vec::with_capacity(48);
                
                for i in 0..*num_children as usize {
                    child_index[keys[i] as usize] = i as u8;
                    new_children.push(children[i].take());
                }
                
                *self = CompactNode::Node48 {
                    prefix: std::mem::take(prefix),
                    num_children: *num_children,
                    child_index,
                    children: new_children,
                    leaf_value: std::mem::take(leaf_value),
                };
                
                stats.node16_count = stats.node16_count.saturating_sub(1);
                stats.node48_count += 1;
            }
            CompactNode::Node48 { prefix, child_index, num_children, children, leaf_value } => {
                let mut new_children: Box<[Option<Box<CompactNode<V>>>; 256]> = 
                    Box::new(std::array::from_fn(|_| None));
                
                for (byte, &idx) in child_index.iter().enumerate() {
                    if idx != 255 && (idx as usize) < children.len() {
                        new_children[byte] = children[idx as usize].take();
                    }
                }
                
                *self = CompactNode::Node256 {
                    prefix: std::mem::take(prefix),
                    num_children: *num_children as u16,
                    children: new_children,
                    leaf_value: std::mem::take(leaf_value),
                };
                
                stats.node48_count = stats.node48_count.saturating_sub(1);
                stats.node256_count += 1;
            }
            _ => {}
        }
    }
    
    pub fn add_child_grow(&mut self, key: u8, child: Box<CompactNode<V>>, stats: &mut CompactArtStats) {
        if self.should_grow() {
            self.grow(stats);
        }
        self.add_child(key, child);
    }
    
    pub fn leaf_value(&self) -> Option<&(KeyRef, V)> {
        match self {
            CompactNode::Leaf { .. } => None,
            CompactNode::Node4 { leaf_value, .. }
            | CompactNode::Node16 { leaf_value, .. }
            | CompactNode::Node48 { leaf_value, .. }
            | CompactNode::Node256 { leaf_value, .. } => leaf_value.as_ref(),
        }
    }
    
    pub fn set_leaf_value(&mut self, value: Option<(KeyRef, V)>) {
        match self {
            CompactNode::Leaf { .. } => {}
            CompactNode::Node4 { leaf_value, .. }
            | CompactNode::Node16 { leaf_value, .. }
            | CompactNode::Node48 { leaf_value, .. }
            | CompactNode::Node256 { leaf_value, .. } => {
                *leaf_value = value;
            }
        }
    }
    
    pub fn take_leaf_value(&mut self) -> Option<(KeyRef, V)> {
        match self {
            CompactNode::Leaf { .. } => None,
            CompactNode::Node4 { leaf_value, .. }
            | CompactNode::Node16 { leaf_value, .. }
            | CompactNode::Node48 { leaf_value, .. }
            | CompactNode::Node256 { leaf_value, .. } => leaf_value.take(),
        }
    }
}

/// A Compact Adaptive Radix Tree with arena-backed key storage.
pub struct CompactArt<V> {
    root: Option<Box<CompactNode<V>>>,
    key_arena: KeyArena,
    size: usize,
    stats: CompactArtStats,
}

impl<V> CompactArt<V> {
    pub fn new() -> Self {
        Self {
            root: None,
            key_arena: KeyArena::new(),
            size: 0,
            stats: CompactArtStats::default(),
        }
    }
    
    /// Insert a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V>
    where
        V: Clone,
    {
        // Store key in arena
        let key_ref = self.key_arena.store(key);
        
        if self.root.is_none() {
            self.root = Some(Box::new(CompactNode::new_leaf(key_ref, value)));
            self.size = 1;
            self.stats.leaf_count = 1;
            return None;
        }
        
        let root = self.root.take().unwrap();
        let (new_root, old_value) = self.insert_recursive(root, key, key_ref, 0, value);
        self.root = Some(new_root);
        
        if old_value.is_none() {
            self.size += 1;
        }
        old_value
    }
    
    fn insert_recursive(
        &mut self,
        mut node: Box<CompactNode<V>>,
        key: &[u8],
        key_ref: KeyRef,
        depth: usize,
        value: V,
    ) -> (Box<CompactNode<V>>, Option<V>)
    where
        V: Clone,
    {
        match &mut *node {
            CompactNode::Leaf { key_ref: leaf_key_ref, value: leaf_value } => {
                let leaf_key = self.key_arena.get(*leaf_key_ref);
                
                if leaf_key == key {
                    // Replace value
                    let old = std::mem::replace(leaf_value, value);
                    return (node, Some(old));
                }
                
                // Keys differ, need to split
                let common_prefix_len = leaf_key[depth..]
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                let split_depth = depth + common_prefix_len;
                let mut new_inner = Box::new(CompactNode::new_node4());
                
                let existing_byte = leaf_key.get(split_depth).copied();
                let new_byte = key.get(split_depth).copied();
                
                match (existing_byte, new_byte) {
                    (Some(eb), Some(nb)) => {
                        let new_leaf = Box::new(CompactNode::new_leaf(key_ref, value));
                        new_inner.add_child(eb, node);
                        new_inner.add_child(nb, new_leaf);
                        self.stats.leaf_count += 1;
                    }
                    (Some(eb), None) => {
                        // New key is prefix of existing
                        new_inner.add_child(eb, node);
                        new_inner.set_leaf_value(Some((key_ref, value)));
                        self.stats.internal_leaf_count += 1;
                    }
                    (None, Some(nb)) => {
                        // Existing is prefix of new
                        // The existing leaf becomes the value at the new internal node
                        // The new key gets a child
                        let old_key_ref = *leaf_key_ref;
                        if let CompactNode::Leaf { value: old_value, .. } = *node {
                            // Move old value to internal node's leaf_value
                            new_inner.set_leaf_value(Some((old_key_ref, old_value)));
                            // Create new leaf for the new key
                            let new_leaf = Box::new(CompactNode::new_leaf(key_ref, value));
                            new_inner.add_child(nb, new_leaf);
                            self.stats.leaf_count += 1;
                            self.stats.internal_leaf_count += 1;
                        }
                    }
                    (None, None) => unreachable!(),
                }
                
                if common_prefix_len > 0 {
                    new_inner.set_prefix(&key[depth..split_depth]);
                }
                
                self.stats.node4_count += 1;
                (new_inner, None)
            }
            
            CompactNode::Node4 { .. }
            | CompactNode::Node16 { .. }
            | CompactNode::Node48 { .. }
            | CompactNode::Node256 { .. } => {
                let prefix = node.prefix().to_vec();
                let prefix_len = prefix.len();
                
                let prefix_match = prefix
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                if prefix_match < prefix_len {
                    // Prefix mismatch - split
                    let mut new_inner = Box::new(CompactNode::new_node4());
                    new_inner.set_prefix(&prefix[..prefix_match]);
                    
                    let old_prefix_byte = prefix[prefix_match];
                    node.set_prefix(&prefix[prefix_match + 1..]);
                    
                    new_inner.add_child(old_prefix_byte, node);
                    
                    let new_key_depth = depth + prefix_match;
                    if new_key_depth < key.len() {
                        let new_byte = key[new_key_depth];
                        let new_leaf = Box::new(CompactNode::new_leaf(key_ref, value));
                        new_inner.add_child(new_byte, new_leaf);
                        self.stats.leaf_count += 1;
                    } else {
                        new_inner.set_leaf_value(Some((key_ref, value)));
                        self.stats.internal_leaf_count += 1;
                    }
                    
                    self.stats.node4_count += 1;
                    return (new_inner, None);
                }
                
                let next_depth = depth + prefix_len;
                
                if next_depth >= key.len() {
                    // Key ends at this node
                    if let Some((_, ref old_val)) = node.leaf_value() {
                        let old = old_val.clone();
                        node.set_leaf_value(Some((key_ref, value)));
                        return (node, Some(old));
                    }
                    node.set_leaf_value(Some((key_ref, value)));
                    self.stats.internal_leaf_count += 1;
                    return (node, None);
                }
                
                let next_byte = key[next_depth];
                
                if let Some(child_idx) = node.find_child(next_byte) {
                    let child = node.remove_child(child_idx);
                    let (new_child, old_value) = self.insert_recursive(child, key, key_ref, next_depth + 1, value);
                    node.add_child(next_byte, new_child);
                    (node, old_value)
                } else {
                    let new_leaf = Box::new(CompactNode::new_leaf(key_ref, value));
                    node.add_child_grow(next_byte, new_leaf, &mut self.stats);
                    self.stats.leaf_count += 1;
                    (node, None)
                }
            }
        }
    }
    
    /// Get a reference to the value for a key.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        let mut node = self.root.as_ref()?;
        let mut depth = 0;
        
        loop {
            match &**node {
                CompactNode::Leaf { key_ref, value } => {
                    let stored_key = self.key_arena.get(*key_ref);
                    if stored_key == key {
                        return Some(value);
                    }
                    return None;
                }
                
                CompactNode::Node4 { prefix, children, leaf_value, num_children, keys, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            let stored_key = self.key_arena.get(*key_ref);
                            if stored_key == key {
                                return Some(value);
                            }
                        }
                        return None;
                    }
                    
                    let next_byte = key[depth];
                    let mut found = None;
                    for i in 0..*num_children as usize {
                        if keys[i] == next_byte {
                            found = Some(i);
                            break;
                        }
                    }
                    
                    if let Some(idx) = found {
                        node = children[idx].as_ref()?;
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                CompactNode::Node16 { prefix, children, leaf_value, num_children, keys, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            let stored_key = self.key_arena.get(*key_ref);
                            if stored_key == key {
                                return Some(value);
                            }
                        }
                        return None;
                    }
                    
                    let next_byte = key[depth];
                    let mut found = None;
                    for i in 0..*num_children as usize {
                        if keys[i] == next_byte {
                            found = Some(i);
                            break;
                        }
                    }
                    
                    if let Some(idx) = found {
                        node = children[idx].as_ref()?;
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                CompactNode::Node48 { prefix, children, leaf_value, child_index, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            let stored_key = self.key_arena.get(*key_ref);
                            if stored_key == key {
                                return Some(value);
                            }
                        }
                        return None;
                    }
                    
                    let next_byte = key[depth];
                    let idx = child_index[next_byte as usize];
                    if idx != 255 {
                        node = children[idx as usize].as_ref()?;
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                CompactNode::Node256 { prefix, children, leaf_value, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            let stored_key = self.key_arena.get(*key_ref);
                            if stored_key == key {
                                return Some(value);
                            }
                        }
                        return None;
                    }
                    
                    let next_byte = key[depth];
                    if let Some(child) = &children[next_byte as usize] {
                        node = child;
                        depth += 1;
                    } else {
                        return None;
                    }
                }
            }
        }
    }
    
    /// Check if a key exists.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }
    
    /// Remove a key and return its value.
    pub fn remove(&mut self, key: &[u8]) -> Option<V>
    where
        V: Clone,
    {
        if self.root.is_none() {
            return None;
        }
        
        let root = self.root.take().unwrap();
        let (new_root, old_value) = self.remove_recursive(root, key, 0);
        self.root = new_root;
        
        if old_value.is_some() {
            self.size -= 1;
        }
        old_value
    }
    
    fn remove_recursive(
        &self,
        mut node: Box<CompactNode<V>>,
        key: &[u8],
        depth: usize,
    ) -> (Option<Box<CompactNode<V>>>, Option<V>)
    where
        V: Clone,
    {
        match &mut *node {
            CompactNode::Leaf { key_ref, value } => {
                let stored_key = self.key_arena.get(*key_ref);
                if stored_key == key {
                    (None, Some(value.clone()))
                } else {
                    (Some(node), None)
                }
            }
            
            _ => {
                let prefix = node.prefix().to_vec();
                let prefix_len = prefix.len();
                
                if key.len() < depth + prefix_len {
                    return (Some(node), None);
                }
                if &key[depth..depth + prefix_len] != prefix.as_slice() {
                    return (Some(node), None);
                }
                
                let next_depth = depth + prefix_len;
                
                if next_depth >= key.len() {
                    // Check leaf value
                    let should_remove = if let Some((key_ref, _)) = node.leaf_value() {
                        self.key_arena.get(*key_ref) == key
                    } else {
                        false
                    };
                    
                    if should_remove {
                        if let Some((_, value)) = node.take_leaf_value() {
                            return (Some(node), Some(value));
                        }
                    }
                    return (Some(node), None);
                }
                
                let next_byte = key[next_depth];
                
                if let Some(child_idx) = node.find_child(next_byte) {
                    let child = node.remove_child(child_idx);
                    let (new_child, old_value) = self.remove_recursive(child, key, next_depth + 1);
                    
                    if let Some(c) = new_child {
                        node.add_child(next_byte, c);
                    }
                    
                    if node.num_children() == 0 && node.leaf_value().is_none() {
                        return (None, old_value);
                    }
                    
                    (Some(node), old_value)
                } else {
                    (Some(node), None)
                }
            }
        }
    }
    
    /// Get the number of keys.
    pub fn len(&self) -> usize {
        self.size
    }
    
    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    
    /// Get memory statistics.
    pub fn memory_stats(&self) -> CompactArtStats {
        let mut stats = self.stats.clone();
        stats.key_arena_bytes = self.key_arena.len();
        
        // Recompute node bytes
        stats.node_bytes = 0;
        if let Some(ref root) = self.root {
            Self::compute_node_bytes(root, &mut stats.node_bytes);
        }
        
        stats
    }
    
    fn compute_node_bytes(node: &CompactNode<V>, bytes: &mut usize) {
        match node {
            CompactNode::Leaf { .. } => {
                // KeyRef (6 bytes) + value
                *bytes += 6 + std::mem::size_of::<V>();
            }
            CompactNode::Node4 { prefix, children, .. } => {
                *bytes += 24 + prefix.capacity() + 4 + 4 * 8 + 8; // prefix Vec + keys + children + leaf_value
                for child in children.iter().flatten() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
            CompactNode::Node16 { prefix, children, .. } => {
                *bytes += 24 + prefix.capacity() + 16 + 16 * 8 + 8;
                for child in children.iter().flatten() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
            CompactNode::Node48 { prefix, children, .. } => {
                *bytes += 24 + prefix.capacity() + 256 + 48 * 8 + 8;
                for child in children.iter().flatten() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
            CompactNode::Node256 { prefix, children, .. } => {
                *bytes += 24 + prefix.capacity() + 256 * 8 + 8;
                for child in children.iter().flatten() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
        }
    }
    
    /// Iterate over a range of keys.
    pub fn range<R>(&self, range: R) -> Vec<(Vec<u8>, V)>
    where
        R: RangeBounds<Vec<u8>>,
        V: Clone,
    {
        let start = match range.start_bound() {
            Bound::Included(s) => Bound::Included(s.clone()),
            Bound::Excluded(s) => Bound::Excluded(s.clone()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let end = match range.end_bound() {
            Bound::Included(e) => Bound::Included(e.clone()),
            Bound::Excluded(e) => Bound::Excluded(e.clone()),
            Bound::Unbounded => Bound::Unbounded,
        };
        
        let mut results = Vec::new();
        if let Some(ref root) = self.root {
            self.collect_range(root, &start, &end, &mut results);
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
    
    fn in_range(key: &[u8], start: &Bound<Vec<u8>>, end: &Bound<Vec<u8>>) -> bool {
        let after_start = match start {
            Bound::Included(s) => key >= s.as_slice(),
            Bound::Excluded(s) => key > s.as_slice(),
            Bound::Unbounded => true,
        };
        let before_end = match end {
            Bound::Included(e) => key <= e.as_slice(),
            Bound::Excluded(e) => key < e.as_slice(),
            Bound::Unbounded => true,
        };
        after_start && before_end
    }
    
    fn collect_range(
        &self,
        node: &CompactNode<V>,
        start: &Bound<Vec<u8>>,
        end: &Bound<Vec<u8>>,
        results: &mut Vec<(Vec<u8>, V)>,
    ) where
        V: Clone,
    {
        match node {
            CompactNode::Leaf { key_ref, value } => {
                let key = self.key_arena.get(*key_ref);
                if Self::in_range(key, start, end) {
                    results.push((key.to_vec(), value.clone()));
                }
            }
            CompactNode::Node4 { children, leaf_value, num_children, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.key_arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for i in 0..*num_children as usize {
                    if let Some(ref child) = children[i] {
                        self.collect_range(child, start, end, results);
                    }
                }
            }
            CompactNode::Node16 { children, leaf_value, num_children, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.key_arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for i in 0..*num_children as usize {
                    if let Some(ref child) = children[i] {
                        self.collect_range(child, start, end, results);
                    }
                }
            }
            CompactNode::Node48 { children, leaf_value, child_index, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.key_arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for byte in 0..=255u8 {
                    let idx = child_index[byte as usize];
                    if idx != 255 && (idx as usize) < children.len() {
                        if let Some(ref child) = children[idx as usize] {
                            self.collect_range(child, start, end, results);
                        }
                    }
                }
            }
            CompactNode::Node256 { children, leaf_value, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.key_arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for child in children.iter().flatten() {
                    self.collect_range(child, start, end, results);
                }
            }
        }
    }
    
    /// Prefix scan.
    pub fn prefix_scan(&self, prefix: &[u8]) -> Vec<(Vec<u8>, V)>
    where
        V: Clone,
    {
        let start = prefix.to_vec();
        let end = Self::prefix_end(prefix);
        
        self.range(match end {
            Some(e) => (Bound::Included(start), Bound::Excluded(e)),
            None => (Bound::Included(start), Bound::Unbounded),
        })
    }
    
    fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
        let mut end = prefix.to_vec();
        while let Some(last) = end.pop() {
            if last < 255 {
                end.push(last + 1);
                return Some(end);
            }
        }
        None
    }
}

impl<V> Default for CompactArt<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_compact_art_basic() {
        let mut tree: CompactArt<u64> = CompactArt::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        tree.insert(b"help", 3);
        
        assert_eq!(tree.get(b"hello"), Some(&1));
        assert_eq!(tree.get(b"world"), Some(&2));
        assert_eq!(tree.get(b"help"), Some(&3));
        assert_eq!(tree.get(b"hell"), None);
    }
    
    #[test]
    fn test_compact_art_update() {
        let mut tree: CompactArt<u64> = CompactArt::new();
        
        assert!(tree.insert(b"key", 1).is_none());
        assert_eq!(tree.insert(b"key", 2), Some(1));
        assert_eq!(tree.get(b"key"), Some(&2));
    }
    
    #[test]
    fn test_compact_art_many_keys() {
        let mut tree: CompactArt<u64> = CompactArt::new();
        
        for i in 0..1000u64 {
            let key = format!("key:{:08}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        assert_eq!(tree.len(), 1000);
        
        for i in 0..1000u64 {
            let key = format!("key:{:08}", i);
            assert_eq!(tree.get(key.as_bytes()), Some(&i));
        }
    }
    
    #[test]
    fn test_compact_art_prefix_scan() {
        let mut tree: CompactArt<u64> = CompactArt::new();
        
        tree.insert(b"user:1001", 1);
        tree.insert(b"user:1002", 2);
        tree.insert(b"user:1003", 3);
        tree.insert(b"post:1001", 100);
        
        let users = tree.prefix_scan(b"user:");
        assert_eq!(users.len(), 3);
    }
    
    #[test]
    fn test_key_ref_size() {
        // Verify our KeyRef is compact
        assert_eq!(std::mem::size_of::<KeyRef>(), 6);
    }
}
