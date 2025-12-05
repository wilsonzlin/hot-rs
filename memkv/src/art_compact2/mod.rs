//! Ultra-Compact ART with arena-backed keys AND prefixes.
//!
//! This version stores both keys and prefixes in a shared arena:
//! - Keys: 6-byte KeyRef instead of 24-byte `Vec<u8>`
//! - Prefixes: 6-byte KeyRef instead of 24-byte `Vec<u8>`
//! - Even more memory efficient than CompactArt

use std::ops::{Bound, RangeBounds};

/// A 32-bit offset into the arena (shared for keys and prefixes).
/// Uses packed representation (6 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(C, packed)]
pub struct DataRef {
    offset: u32,
    len: u16,
}

impl DataRef {
    pub const fn empty() -> Self {
        Self { offset: 0, len: 0 }
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

/// Shared arena for keys and prefixes.
pub struct DataArena {
    data: Vec<u8>,
}

impl DataArena {
    pub fn new() -> Self {
        Self::with_capacity(64 * 1024)
    }
    
    pub fn with_capacity(cap: usize) -> Self {
        Self { data: Vec::with_capacity(cap) }
    }
    
    /// Store data and return its reference.
    pub fn store(&mut self, bytes: &[u8]) -> DataRef {
        if bytes.is_empty() {
            return DataRef::empty();
        }
        let offset = self.data.len();
        self.data.extend_from_slice(bytes);
        DataRef::new(offset, bytes.len())
    }
    
    /// Get data by reference.
    pub fn get(&self, data_ref: DataRef) -> &[u8] {
        if data_ref.is_empty() {
            return &[];
        }
        let start = data_ref.offset();
        let end = start + data_ref.len();
        &self.data[start..end]
    }
    
    /// Total bytes used.
    pub fn len(&self) -> usize {
        self.data.len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl Default for DataArena {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory statistics.
#[derive(Debug, Clone, Default)]
pub struct UltraCompactStats {
    /// Bytes in data arena
    pub arena_bytes: usize,
    /// Bytes for node structures (estimated)
    pub node_bytes: usize,
    /// Node counts
    pub node4_count: usize,
    pub node16_count: usize,
    pub node48_count: usize,
    pub node256_count: usize,
    pub leaf_count: usize,
}

/// A node in the Ultra-Compact ART.
/// Uses DataRef for both keys (in leaves) and prefixes (in internal nodes).
/// Uses Vec for children to keep enum size small.
pub enum UltraNode<V> {
    /// A leaf node.
    Leaf {
        key_ref: DataRef,
        value: V,
    },
    
    /// Node with 1-4 children.
    Node4 {
        prefix_ref: DataRef,
        num_children: u8,
        keys: [u8; 4],
        children: Vec<Box<UltraNode<V>>>,
        leaf_value: Option<(DataRef, V)>,
    },
    
    /// Node with 5-16 children.
    Node16 {
        prefix_ref: DataRef,
        num_children: u8,
        keys: [u8; 16],
        children: Vec<Box<UltraNode<V>>>,
        leaf_value: Option<(DataRef, V)>,
    },
    
    /// Node with 17-48 children.
    Node48 {
        prefix_ref: DataRef,
        num_children: u8,
        child_index: Box<[u8; 256]>,
        children: Vec<Box<UltraNode<V>>>,
        leaf_value: Option<(DataRef, V)>,
    },
    
    /// Node with 49-256 children.
    Node256 {
        prefix_ref: DataRef,
        num_children: u16,
        children: Box<[Option<Box<UltraNode<V>>>; 256]>,
        leaf_value: Option<(DataRef, V)>,
    },
}

impl<V> UltraNode<V> {
    pub fn new_leaf(key_ref: DataRef, value: V) -> Self {
        UltraNode::Leaf { key_ref, value }
    }
    
    pub fn new_node4() -> Self {
        UltraNode::Node4 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            keys: [0; 4],
            children: Vec::with_capacity(4),
            leaf_value: None,
        }
    }
    
    pub fn new_node16() -> Self {
        UltraNode::Node16 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            keys: [0; 16],
            children: Vec::with_capacity(16),
            leaf_value: None,
        }
    }
    
    pub fn new_node48() -> Self {
        UltraNode::Node48 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            child_index: Box::new([255; 256]),
            children: Vec::with_capacity(48),
            leaf_value: None,
        }
    }
    
    pub fn new_node256() -> Self {
        UltraNode::Node256 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            children: Box::new(std::array::from_fn(|_| None)),
            leaf_value: None,
        }
    }
    
    pub fn prefix_ref(&self) -> DataRef {
        match self {
            UltraNode::Leaf { .. } => DataRef::empty(),
            UltraNode::Node4 { prefix_ref, .. }
            | UltraNode::Node16 { prefix_ref, .. }
            | UltraNode::Node48 { prefix_ref, .. }
            | UltraNode::Node256 { prefix_ref, .. } => *prefix_ref,
        }
    }
    
    pub fn set_prefix_ref(&mut self, new_ref: DataRef) {
        match self {
            UltraNode::Leaf { .. } => {}
            UltraNode::Node4 { prefix_ref, .. }
            | UltraNode::Node16 { prefix_ref, .. }
            | UltraNode::Node48 { prefix_ref, .. }
            | UltraNode::Node256 { prefix_ref, .. } => {
                *prefix_ref = new_ref;
            }
        }
    }
    
    pub fn num_children(&self) -> usize {
        match self {
            UltraNode::Leaf { .. } => 0,
            UltraNode::Node4 { num_children, .. } => *num_children as usize,
            UltraNode::Node16 { num_children, .. } => *num_children as usize,
            UltraNode::Node48 { num_children, .. } => *num_children as usize,
            UltraNode::Node256 { num_children, .. } => *num_children as usize,
        }
    }
    
    pub fn find_child(&self, key: u8) -> Option<usize> {
        match self {
            UltraNode::Leaf { .. } => None,
            UltraNode::Node4 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(i);
                    }
                }
                None
            }
            UltraNode::Node16 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(i);
                    }
                }
                None
            }
            UltraNode::Node48 { child_index, .. } => {
                let idx = child_index[key as usize];
                if idx != 255 {
                    Some(idx as usize)
                } else {
                    None
                }
            }
            UltraNode::Node256 { children, .. } => {
                if children[key as usize].is_some() {
                    Some(key as usize)
                } else {
                    None
                }
            }
        }
    }
    
    pub fn add_child(&mut self, key: u8, child: Box<UltraNode<V>>) {
        match self {
            UltraNode::Leaf { .. } => panic!("Cannot add child to leaf"),
            UltraNode::Node4 { keys, num_children, children, .. } => {
                // Check if key exists and replace
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = child;
                        return;
                    }
                }
                // Add new child
                if (*num_children as usize) < 4 {
                    let idx = *num_children as usize;
                    keys[idx] = key;
                    if idx < children.len() {
                        children[idx] = child;
                    } else {
                        children.push(child);
                    }
                    *num_children += 1;
                } else {
                    panic!("Node4 is full");
                }
            }
            UltraNode::Node16 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = child;
                        return;
                    }
                }
                if (*num_children as usize) < 16 {
                    let idx = *num_children as usize;
                    keys[idx] = key;
                    if idx < children.len() {
                        children[idx] = child;
                    } else {
                        children.push(child);
                    }
                    *num_children += 1;
                } else {
                    panic!("Node16 is full");
                }
            }
            UltraNode::Node48 { child_index, num_children, children, .. } => {
                let existing_idx = child_index[key as usize];
                if existing_idx != 255 && (existing_idx as usize) < children.len() {
                    children[existing_idx as usize] = child;
                } else if (*num_children as usize) < 48 {
                    // IMPORTANT: Reuse existing slots when possible to prevent Vec from growing unboundedly
                    let slot = if children.len() < 48 {
                        let slot = children.len();
                        children.push(child);
                        slot
                    } else {
                        // Build bitmap of used slots - O(256) instead of O(48 * 256)
                        let mut used = [false; 48];
                        for &idx in child_index.iter() {
                            if idx != 255 && (idx as usize) < 48 {
                                used[idx as usize] = true;
                            }
                        }
                        // Find first free slot - O(48)
                        let mut free_slot = None;
                        for i in 0..children.len().min(48) {
                            if !used[i] {
                                free_slot = Some(i);
                                break;
                            }
                        }
                        if let Some(slot) = free_slot {
                            children[slot] = child;
                            slot
                        } else {
                            // This shouldn't happen if num_children < 48
                            let slot = children.len();
                            children.push(child);
                            slot
                        }
                    };
                    debug_assert!(slot <= 255, "Node48 slot overflow");
                    child_index[key as usize] = slot as u8;
                    *num_children += 1;
                } else {
                    panic!("Node48 is full");
                }
            }
            UltraNode::Node256 { children, num_children, .. } => {
                if children[key as usize].is_none() {
                    *num_children += 1;
                }
                children[key as usize] = Some(child);
            }
        }
    }
    
    pub fn remove_child(&mut self, idx: usize) -> Box<UltraNode<V>> {
        match self {
            UltraNode::Leaf { .. } => panic!("Cannot remove from leaf"),
            UltraNode::Node4 { keys, num_children, children, .. } => {
                let child = std::mem::replace(&mut children[idx], Box::new(UltraNode::new_node4()));
                for i in idx..(*num_children as usize - 1) {
                    keys[i] = keys[i + 1];
                    children.swap(i, i + 1);
                }
                *num_children -= 1;
                child
            }
            UltraNode::Node16 { keys, num_children, children, .. } => {
                let child = std::mem::replace(&mut children[idx], Box::new(UltraNode::new_node4()));
                for i in idx..(*num_children as usize - 1) {
                    keys[i] = keys[i + 1];
                    children.swap(i, i + 1);
                }
                *num_children -= 1;
                child
            }
            UltraNode::Node48 { child_index, num_children, children, .. } => {
                let mut key_byte = None;
                for (k, &i) in child_index.iter().enumerate() {
                    if i != 255 && i as usize == idx {
                        key_byte = Some(k);
                        break;
                    }
                }
                let child = std::mem::replace(&mut children[idx], Box::new(UltraNode::new_node4()));
                if let Some(kb) = key_byte {
                    child_index[kb] = 255;
                    *num_children -= 1;
                }
                child
            }
            UltraNode::Node256 { children, num_children, .. } => {
                *num_children -= 1;
                children[idx].take().expect("Child should exist")
            }
        }
    }
    
    pub fn should_grow(&self) -> bool {
        match self {
            UltraNode::Leaf { .. } => false,
            UltraNode::Node4 { num_children, .. } => *num_children >= 4,
            UltraNode::Node16 { num_children, .. } => *num_children >= 16,
            UltraNode::Node48 { num_children, .. } => *num_children >= 48,
            UltraNode::Node256 { .. } => false,
        }
    }
    
    pub fn grow(&mut self, stats: &mut UltraCompactStats) {
        match self {
            UltraNode::Node4 { prefix_ref, keys, num_children, children, leaf_value } => {
                let mut new_keys = [0u8; 16];
                new_keys[..4].copy_from_slice(keys);
                
                *self = UltraNode::Node16 {
                    prefix_ref: *prefix_ref,
                    num_children: *num_children,
                    keys: new_keys,
                    children: std::mem::take(children),
                    leaf_value: std::mem::take(leaf_value),
                };
                
                stats.node4_count = stats.node4_count.saturating_sub(1);
                stats.node16_count += 1;
            }
            UltraNode::Node16 { prefix_ref, keys, num_children, children, leaf_value } => {
                let mut child_index = Box::new([255u8; 256]);
                
                for i in 0..*num_children as usize {
                    child_index[keys[i] as usize] = i as u8;
                }
                
                *self = UltraNode::Node48 {
                    prefix_ref: *prefix_ref,
                    num_children: *num_children,
                    child_index,
                    children: std::mem::take(children),
                    leaf_value: std::mem::take(leaf_value),
                };
                
                stats.node16_count = stats.node16_count.saturating_sub(1);
                stats.node48_count += 1;
            }
            UltraNode::Node48 { prefix_ref, child_index, num_children, children, leaf_value } => {
                let mut new_children: Box<[Option<Box<UltraNode<V>>>; 256]> = 
                    Box::new(std::array::from_fn(|_| None));
                
                for (byte, &idx) in child_index.iter().enumerate() {
                    if idx != 255 && (idx as usize) < children.len() {
                        new_children[byte] = Some(std::mem::replace(
                            &mut children[idx as usize],
                            Box::new(UltraNode::new_node4())
                        ));
                    }
                }
                
                *self = UltraNode::Node256 {
                    prefix_ref: *prefix_ref,
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
    
    pub fn add_child_grow(&mut self, key: u8, child: Box<UltraNode<V>>, stats: &mut UltraCompactStats) {
        if self.should_grow() {
            self.grow(stats);
        }
        self.add_child(key, child);
    }
    
    pub fn leaf_value(&self) -> Option<&(DataRef, V)> {
        match self {
            UltraNode::Leaf { .. } => None,
            UltraNode::Node4 { leaf_value, .. }
            | UltraNode::Node16 { leaf_value, .. }
            | UltraNode::Node48 { leaf_value, .. }
            | UltraNode::Node256 { leaf_value, .. } => leaf_value.as_ref(),
        }
    }
    
    pub fn set_leaf_value(&mut self, value: Option<(DataRef, V)>) {
        match self {
            UltraNode::Leaf { .. } => {}
            UltraNode::Node4 { leaf_value, .. }
            | UltraNode::Node16 { leaf_value, .. }
            | UltraNode::Node48 { leaf_value, .. }
            | UltraNode::Node256 { leaf_value, .. } => {
                *leaf_value = value;
            }
        }
    }
    
    pub fn take_leaf_value(&mut self) -> Option<(DataRef, V)> {
        match self {
            UltraNode::Leaf { .. } => None,
            UltraNode::Node4 { leaf_value, .. }
            | UltraNode::Node16 { leaf_value, .. }
            | UltraNode::Node48 { leaf_value, .. }
            | UltraNode::Node256 { leaf_value, .. } => leaf_value.take(),
        }
    }
}

/// Ultra-Compact ART with arena-backed keys and prefixes.
pub struct UltraCompactArt<V> {
    root: Option<Box<UltraNode<V>>>,
    arena: DataArena,
    size: usize,
    stats: UltraCompactStats,
}

impl<V> UltraCompactArt<V> {
    pub fn new() -> Self {
        Self {
            root: None,
            arena: DataArena::new(),
            size: 0,
            stats: UltraCompactStats::default(),
        }
    }
    
    
    /// Insert a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V>
    where
        V: Clone,
    {
        let key_ref = self.arena.store(key);
        
        if self.root.is_none() {
            self.root = Some(Box::new(UltraNode::new_leaf(key_ref, value)));
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
        mut node: Box<UltraNode<V>>,
        key: &[u8],
        key_ref: DataRef,
        depth: usize,
        value: V,
    ) -> (Box<UltraNode<V>>, Option<V>)
    where
        V: Clone,
    {
        match &mut *node {
            UltraNode::Leaf { key_ref: leaf_key_ref, value: leaf_value } => {
                // Get leaf key data we need before any mutable borrows
                let leaf_key_ref_copy = *leaf_key_ref;
                let (keys_equal, common_prefix_len, existing_byte, new_byte) = {
                    let leaf_key = self.arena.get(leaf_key_ref_copy);
                    
                    if leaf_key == key {
                        let old = std::mem::replace(leaf_value, value);
                        return (node, Some(old));
                    }
                    
                    let common_prefix_len = leaf_key[depth..]
                        .iter()
                        .zip(key[depth..].iter())
                        .take_while(|(a, b)| a == b)
                        .count();
                    
                    let split_depth = depth + common_prefix_len;
                    let existing_byte = leaf_key.get(split_depth).copied();
                    let new_byte = key.get(split_depth).copied();
                    
                    (false, common_prefix_len, existing_byte, new_byte)
                };
                let _ = keys_equal; // suppress warning
                
                let split_depth = depth + common_prefix_len;
                let mut new_inner = Box::new(UltraNode::new_node4());
                
                // Store prefix in arena
                if common_prefix_len > 0 {
                    let prefix_ref = self.arena.store(&key[depth..split_depth]);
                    new_inner.set_prefix_ref(prefix_ref);
                }
                
                match (existing_byte, new_byte) {
                    (Some(eb), Some(nb)) => {
                        let new_leaf = Box::new(UltraNode::new_leaf(key_ref, value));
                        new_inner.add_child(eb, node);
                        new_inner.add_child(nb, new_leaf);
                        self.stats.leaf_count += 1;
                    }
                    (Some(eb), None) => {
                        new_inner.add_child(eb, node);
                        new_inner.set_leaf_value(Some((key_ref, value)));
                    }
                    (None, Some(nb)) => {
                        if let UltraNode::Leaf { value: old_value, .. } = *node {
                            new_inner.set_leaf_value(Some((leaf_key_ref_copy, old_value)));
                            let new_leaf = Box::new(UltraNode::new_leaf(key_ref, value));
                            new_inner.add_child(nb, new_leaf);
                            self.stats.leaf_count += 1;
                        }
                    }
                    (None, None) => {
                        // This can happen if keys are identical but were incorrectly detected as different
                        // This shouldn't happen - if we get here, treat as duplicate key
                        eprintln!("WARNING: (None, None) case - depth={}, key_len={}", depth, key.len());
                        let old = std::mem::replace(leaf_value, value);
                        return (node, Some(old));
                    }
                }
                
                self.stats.node4_count += 1;
                (new_inner, None)
            }
            
            _ => {
                let prefix_ref = node.prefix_ref();
                let prefix = self.arena.get(prefix_ref).to_vec();
                let prefix_len = prefix.len();
                
                let prefix_match = prefix
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                if prefix_match < prefix_len {
                    // Prefix mismatch - split
                    let mut new_inner = Box::new(UltraNode::new_node4());
                    
                    // New node's prefix
                    if prefix_match > 0 {
                        let new_prefix_ref = self.arena.store(&prefix[..prefix_match]);
                        new_inner.set_prefix_ref(new_prefix_ref);
                    }
                    
                    // Update old node's prefix
                    let old_prefix_byte = prefix[prefix_match];
                    let old_remaining_ref = self.arena.store(&prefix[prefix_match + 1..]);
                    node.set_prefix_ref(old_remaining_ref);
                    
                    new_inner.add_child(old_prefix_byte, node);
                    
                    let new_key_depth = depth + prefix_match;
                    if new_key_depth < key.len() {
                        let new_byte = key[new_key_depth];
                        let new_leaf = Box::new(UltraNode::new_leaf(key_ref, value));
                        new_inner.add_child(new_byte, new_leaf);
                        self.stats.leaf_count += 1;
                    } else {
                        new_inner.set_leaf_value(Some((key_ref, value)));
                    }
                    
                    self.stats.node4_count += 1;
                    return (new_inner, None);
                }
                
                let next_depth = depth + prefix_len;
                
                if next_depth >= key.len() {
                    // Key ends here
                    if let Some((_, ref old_val)) = node.leaf_value() {
                        let old = old_val.clone();
                        node.set_leaf_value(Some((key_ref, value)));
                        return (node, Some(old));
                    }
                    node.set_leaf_value(Some((key_ref, value)));
                    return (node, None);
                }
                
                let next_byte = key[next_depth];
                
                if let Some(child_idx) = node.find_child(next_byte) {
                    let child = node.remove_child(child_idx);
                    let (new_child, old_value) = self.insert_recursive(child, key, key_ref, next_depth + 1, value);
                    node.add_child(next_byte, new_child);
                    (node, old_value)
                } else {
                    let new_leaf = Box::new(UltraNode::new_leaf(key_ref, value));
                    node.add_child_grow(next_byte, new_leaf, &mut self.stats);
                    self.stats.leaf_count += 1;
                    (node, None)
                }
            }
        }
    }
    
    /// Get a reference to the value.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        let mut node = self.root.as_ref()?;
        let mut depth = 0;
        
        loop {
            match &**node {
                UltraNode::Leaf { key_ref, value } => {
                    let stored_key = self.arena.get(*key_ref);
                    if stored_key == key {
                        return Some(value);
                    }
                    return None;
                }
                
                UltraNode::Node4 { prefix_ref, children, leaf_value, num_children, keys, .. } => {
                    let prefix = self.arena.get(*prefix_ref);
                    let prefix_len = prefix.len();
                    
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            let stored_key = self.arena.get(*key_ref);
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
                        if idx < children.len() {
                            node = &children[idx];
                            depth += 1;
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                
                UltraNode::Node16 { prefix_ref, children, leaf_value, num_children, keys, .. } => {
                    let prefix = self.arena.get(*prefix_ref);
                    let prefix_len = prefix.len();
                    
                    if key.len() < depth + prefix_len || &key[depth..depth + prefix_len] != prefix {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            if self.arena.get(*key_ref) == key {
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
                        if idx < children.len() {
                            node = &children[idx];
                            depth += 1;
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                
                UltraNode::Node48 { prefix_ref, children, leaf_value, child_index, .. } => {
                    let prefix = self.arena.get(*prefix_ref);
                    let prefix_len = prefix.len();
                    
                    if key.len() < depth + prefix_len || &key[depth..depth + prefix_len] != prefix {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            if self.arena.get(*key_ref) == key {
                                return Some(value);
                            }
                        }
                        return None;
                    }
                    
                    let next_byte = key[depth];
                    let idx = child_index[next_byte as usize];
                    if idx != 255 && (idx as usize) < children.len() {
                        node = &children[idx as usize];
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                UltraNode::Node256 { prefix_ref, children, leaf_value, .. } => {
                    let prefix = self.arena.get(*prefix_ref);
                    let prefix_len = prefix.len();
                    
                    if key.len() < depth + prefix_len || &key[depth..depth + prefix_len] != prefix {
                        return None;
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((key_ref, value)) = leaf_value {
                            if self.arena.get(*key_ref) == key {
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
    
    /// Check if key exists.
    pub fn contains(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }
    
    /// Get number of keys.
    pub fn len(&self) -> usize {
        self.size
    }
    
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    
    /// Get memory statistics.
    pub fn memory_stats(&self) -> UltraCompactStats {
        let mut stats = self.stats.clone();
        stats.arena_bytes = self.arena.len();
        
        // Estimate node bytes
        stats.node_bytes = 0;
        if let Some(ref root) = self.root {
            Self::compute_node_bytes(root, &mut stats.node_bytes);
        }
        
        stats
    }
    
    fn compute_node_bytes(node: &UltraNode<V>, bytes: &mut usize) {
        match node {
            UltraNode::Leaf { .. } => {
                // DataRef (6) + value
                *bytes += 6 + std::mem::size_of::<V>();
            }
            UltraNode::Node4 { children, .. } => {
                // prefix_ref (6) + num_children (1) + keys[4] + Vec (24) + leaf_value
                *bytes += 6 + 1 + 4 + 24 + 16;
                for child in children.iter() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
            UltraNode::Node16 { children, .. } => {
                *bytes += 6 + 1 + 16 + 24 + 16;
                for child in children.iter() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
            UltraNode::Node48 { children, .. } => {
                *bytes += 6 + 1 + 256 + 24 + 16;
                for child in children.iter() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
            UltraNode::Node256 { children, .. } => {
                *bytes += 6 + 2 + 256 * 8 + 16;
                for child in children.iter().flatten() {
                    Self::compute_node_bytes(child, bytes);
                }
            }
        }
    }
    
    /// Range query.
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
        node: &UltraNode<V>,
        start: &Bound<Vec<u8>>,
        end: &Bound<Vec<u8>>,
        results: &mut Vec<(Vec<u8>, V)>,
    ) where
        V: Clone,
    {
        match node {
            UltraNode::Leaf { key_ref, value } => {
                let key = self.arena.get(*key_ref);
                if Self::in_range(key, start, end) {
                    results.push((key.to_vec(), value.clone()));
                }
            }
            UltraNode::Node4 { children, leaf_value, num_children, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for i in 0..*num_children as usize {
                    if i < children.len() {
                        self.collect_range(&children[i], start, end, results);
                    }
                }
            }
            UltraNode::Node16 { children, leaf_value, num_children, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for i in 0..*num_children as usize {
                    if i < children.len() {
                        self.collect_range(&children[i], start, end, results);
                    }
                }
            }
            UltraNode::Node48 { children, leaf_value, child_index, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.arena.get(*key_ref);
                    if Self::in_range(key, start, end) {
                        results.push((key.to_vec(), value.clone()));
                    }
                }
                for byte in 0..=255u8 {
                    let idx = child_index[byte as usize];
                    if idx != 255 && (idx as usize) < children.len() {
                        self.collect_range(&children[idx as usize], start, end, results);
                    }
                }
            }
            UltraNode::Node256 { children, leaf_value, .. } => {
                if let Some((key_ref, value)) = leaf_value {
                    let key = self.arena.get(*key_ref);
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

impl<V> Default for UltraCompactArt<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ultra_compact_basic() {
        let mut tree: UltraCompactArt<u64> = UltraCompactArt::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        tree.insert(b"help", 3);
        
        assert_eq!(tree.get(b"hello"), Some(&1));
        assert_eq!(tree.get(b"world"), Some(&2));
        assert_eq!(tree.get(b"help"), Some(&3));
        assert_eq!(tree.get(b"hell"), None);
    }
    
    #[test]
    fn test_ultra_compact_many() {
        let mut tree: UltraCompactArt<u64> = UltraCompactArt::new();
        
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
    fn test_data_ref_size() {
        assert_eq!(std::mem::size_of::<DataRef>(), 6);
    }
}
