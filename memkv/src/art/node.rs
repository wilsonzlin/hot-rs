//! ART Node types with adaptive sizing.
//!
//! The key insight of ART is using different node layouts based on
//! the actual number of children:
//!
//! - Node4: Up to 4 children (most common, smallest)
//! - Node16: 5-16 children (uses sorted keys for SIMD search)
//! - Node48: 17-48 children (256-byte index + 48 pointers)
//! - Node256: 49-256 children (direct array indexing)

use super::ArtMemoryStats;

/// The type of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// A leaf node containing a key and value.
    Leaf,
    /// A node with up to 4 children.
    Node4,
    /// A node with 5-16 children.
    Node16,
    /// A node with 17-48 children.
    Node48,
    /// A node with 49-256 children.
    Node256,
}

/// A node in the Adaptive Radix Tree.
pub enum Node<V> {
    /// A leaf node storing a key-value pair.
    Leaf {
        /// The full key (for verification on lookup).
        key: Vec<u8>,
        /// The value.
        value: V,
    },

    /// A node with up to 4 children.
    Node4 {
        /// Compressed path prefix.
        prefix: Vec<u8>,
        /// Number of valid children.
        num_children: u8,
        /// Child keys (sorted for search).
        keys: [u8; 4],
        /// Child nodes.
        children: Vec<Box<Node<V>>>,
        /// Optional value if a key ends at this node.
        leaf_value: Option<(Vec<u8>, Box<Node<V>>)>,
    },

    /// A node with 5-16 children.
    Node16 {
        /// Compressed path prefix.
        prefix: Vec<u8>,
        /// Number of valid children.
        num_children: u8,
        /// Child keys (sorted for SIMD search).
        keys: [u8; 16],
        /// Child nodes.
        children: Vec<Box<Node<V>>>,
        /// Optional value if a key ends at this node.
        leaf_value: Option<(Vec<u8>, Box<Node<V>>)>,
    },

    /// A node with 17-48 children.
    Node48 {
        /// Compressed path prefix.
        prefix: Vec<u8>,
        /// Number of valid children.
        num_children: u8,
        /// Index mapping bytes to child positions (255 = empty). Boxed to reduce enum size.
        child_index: Box<[u8; 256]>,
        /// Child nodes.
        children: Vec<Box<Node<V>>>,
        /// Optional value if a key ends at this node.
        leaf_value: Option<(Vec<u8>, Box<Node<V>>)>,
    },

    /// A node with 49-256 children.
    Node256 {
        /// Compressed path prefix.
        prefix: Vec<u8>,
        /// Number of valid children.
        num_children: u16,
        /// Child nodes (direct indexing by byte). Boxed to reduce enum size.
        children: Box<[Option<Box<Node<V>>>; 256]>,
        /// Optional value if a key ends at this node.
        leaf_value: Option<(Vec<u8>, Box<Node<V>>)>,
    },
}

impl<V> Node<V> {
    /// Create a new leaf node.
    pub fn new_leaf(key: Vec<u8>, value: V) -> Self {
        Node::Leaf { key, value }
    }

    /// Create a new Node4.
    pub fn new_node4() -> Self {
        Node::Node4 {
            prefix: Vec::new(),
            num_children: 0,
            keys: [0; 4],
            children: Vec::with_capacity(4),
            leaf_value: None,
        }
    }

    /// Create a new Node16.
    pub fn new_node16() -> Self {
        Node::Node16 {
            prefix: Vec::new(),
            num_children: 0,
            keys: [0; 16],
            children: Vec::with_capacity(16),
            leaf_value: None,
        }
    }

    /// Create a new Node48.
    pub fn new_node48() -> Self {
        Node::Node48 {
            prefix: Vec::new(),
            num_children: 0,
            child_index: Box::new([255; 256]),
            children: Vec::with_capacity(48),
            leaf_value: None,
        }
    }

    /// Create a new Node256.
    pub fn new_node256() -> Self {
        Node::Node256 {
            prefix: Vec::new(),
            num_children: 0,
            children: Box::new(std::array::from_fn(|_| None)),
            leaf_value: None,
        }
    }

    /// Get the node type.
    pub fn node_type(&self) -> NodeType {
        match self {
            Node::Leaf { .. } => NodeType::Leaf,
            Node::Node4 { .. } => NodeType::Node4,
            Node::Node16 { .. } => NodeType::Node16,
            Node::Node48 { .. } => NodeType::Node48,
            Node::Node256 { .. } => NodeType::Node256,
        }
    }

    /// Get the number of children.
    pub fn num_children(&self) -> usize {
        match self {
            Node::Leaf { .. } => 0,
            Node::Node4 { num_children, .. } => *num_children as usize,
            Node::Node16 { num_children, .. } => *num_children as usize,
            Node::Node48 { num_children, .. } => *num_children as usize,
            Node::Node256 { num_children, .. } => *num_children as usize,
        }
    }

    /// Set the prefix.
    pub fn set_prefix(&mut self, new_prefix: &[u8]) {
        match self {
            Node::Leaf { .. } => {}
            Node::Node4 { prefix, .. }
            | Node::Node16 { prefix, .. }
            | Node::Node48 { prefix, .. }
            | Node::Node256 { prefix, .. } => {
                prefix.clear();
                prefix.extend_from_slice(new_prefix);
            }
        }
    }

    /// Get the prefix.
    pub fn prefix(&self) -> &[u8] {
        match self {
            Node::Leaf { .. } => &[],
            Node::Node4 { prefix, .. }
            | Node::Node16 { prefix, .. }
            | Node::Node48 { prefix, .. }
            | Node::Node256 { prefix, .. } => prefix,
        }
    }

    /// Set the leaf value.
    pub fn set_leaf_value(&mut self, value: Option<(Vec<u8>, Box<Node<V>>)>) {
        match self {
            Node::Leaf { .. } => {}
            Node::Node4 { leaf_value, .. }
            | Node::Node16 { leaf_value, .. }
            | Node::Node48 { leaf_value, .. }
            | Node::Node256 { leaf_value, .. } => {
                *leaf_value = value;
            }
        }
    }

    /// Get the leaf value.
    pub fn get_leaf_value(&self) -> Option<&(Vec<u8>, Box<Node<V>>)> {
        match self {
            Node::Leaf { .. } => None,
            Node::Node4 { leaf_value, .. }
            | Node::Node16 { leaf_value, .. }
            | Node::Node48 { leaf_value, .. }
            | Node::Node256 { leaf_value, .. } => leaf_value.as_ref(),
        }
    }

    /// Take the leaf value (remove and return it).
    pub fn take_leaf_value(&mut self) -> Option<(Vec<u8>, Box<Node<V>>)> {
        match self {
            Node::Leaf { .. } => None,
            Node::Node4 { leaf_value, .. }
            | Node::Node16 { leaf_value, .. }
            | Node::Node48 { leaf_value, .. }
            | Node::Node256 { leaf_value, .. } => leaf_value.take(),
        }
    }

    /// Find the index of a child with the given key byte.
    pub fn find_child(&self, key: u8) -> Option<usize> {
        match self {
            Node::Leaf { .. } => None,
            
            Node::Node4 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(i);
                    }
                }
                None
            }
            
            Node::Node16 { keys, num_children, .. } => {
                // Could use SIMD here for performance
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(i);
                    }
                }
                None
            }
            
            Node::Node48 { child_index, .. } => {
                let idx = child_index[key as usize];
                if idx != 255 {
                    Some(idx as usize)
                } else {
                    None
                }
            }
            
            Node::Node256 { children, .. } => {
                if children[key as usize].is_some() {
                    Some(key as usize)
                } else {
                    None
                }
            }
        }
    }

    /// Add a child node.
    pub fn add_child(&mut self, key: u8, child: Box<Node<V>>) {
        match self {
            Node::Leaf { .. } => panic!("Cannot add child to leaf"),
            
            Node::Node4 { keys, num_children, children, .. } => {
                // Check if key already exists
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = child;
                        return;
                    }
                }
                
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
                    panic!("Node4 is full, should grow first");
                }
            }
            
            Node::Node16 { keys, num_children, children, .. } => {
                // Check if key already exists
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
                    panic!("Node16 is full, should grow first");
                }
            }
            
            Node::Node48 { child_index, num_children, children, .. } => {
                let existing_idx = child_index[key as usize];
                if existing_idx != 255 && (existing_idx as usize) < children.len() {
                    // Key exists, replace
                    children[existing_idx as usize] = child;
                } else if (*num_children as usize) < 48 {
                    // Key doesn't exist, add new
                    // First, try to find an empty slot from previous removals
                    let mut free_slot = None;
                    for i in 0..children.len() {
                        let mut in_use = false;
                        for &idx in child_index.iter() {
                            if idx != 255 && idx as usize == i {
                                in_use = true;
                                break;
                            }
                        }
                        if !in_use {
                            free_slot = Some(i);
                            break;
                        }
                    }
                    
                    if let Some(slot) = free_slot {
                        // Reuse empty slot
                        children[slot] = child;
                        child_index[key as usize] = slot as u8;
                        *num_children += 1;
                    } else if children.len() < 48 {
                        // No empty slot, but Vec has room - push
                        let slot = children.len();
                        child_index[key as usize] = slot as u8;
                        children.push(child);
                        *num_children += 1;
                    } else {
                        panic!("Node48: no empty slot found and Vec is full");
                    }
                } else {
                    panic!("Node48 is full, should grow first");
                }
            }
            
            Node::Node256 { children, num_children, .. } => {
                if children[key as usize].is_none() {
                    *num_children += 1;
                }
                children[key as usize] = Some(child);
            }
        }
    }

    /// Add a child, growing the node if necessary.
    pub fn add_child_grow(&mut self, key: u8, child: Box<Node<V>>, stats: &mut ArtMemoryStats) {
        match self {
            Node::Leaf { .. } => panic!("Cannot add child to leaf"),
            
            Node::Node4 { num_children, .. } if *num_children >= 4 => {
                // Grow to Node16
                self.grow_to_node16(stats);
                self.add_child(key, child);
            }
            
            Node::Node16 { num_children, .. } if *num_children >= 16 => {
                // Grow to Node48
                self.grow_to_node48(stats);
                self.add_child(key, child);
            }
            
            Node::Node48 { num_children, .. } if *num_children >= 48 => {
                // Grow to Node256
                self.grow_to_node256(stats);
                self.add_child(key, child);
            }
            
            _ => {
                self.add_child(key, child);
            }
        }
    }

    /// Remove a child and return it.
    pub fn remove_child(&mut self, idx: usize) -> Box<Node<V>> {
        match self {
            Node::Leaf { .. } => panic!("Cannot remove child from leaf"),
            
            Node::Node4 { keys, num_children, children, .. } => {
                let child = std::mem::replace(&mut children[idx], Box::new(Node::new_node4()));
                
                // Compact the array
                for i in idx..(*num_children as usize - 1) {
                    keys[i] = keys[i + 1];
                    children.swap(i, i + 1);
                }
                *num_children -= 1;
                
                child
            }
            
            Node::Node16 { keys, num_children, children, .. } => {
                let child = std::mem::replace(&mut children[idx], Box::new(Node::new_node4()));
                
                // Compact the array
                for i in idx..(*num_children as usize - 1) {
                    keys[i] = keys[i + 1];
                    children.swap(i, i + 1);
                }
                *num_children -= 1;
                
                child
            }
            
            Node::Node48 { child_index, num_children, children, .. } => {
                // Find the key that maps to this index
                let mut key_byte = None;
                for (k, &i) in child_index.iter().enumerate() {
                    if i != 255 && i as usize == idx {
                        key_byte = Some(k);
                        break;
                    }
                }
                
                let child = std::mem::replace(&mut children[idx], Box::new(Node::new_node4()));
                
                if let Some(kb) = key_byte {
                    child_index[kb] = 255; // Mark as empty
                    *num_children -= 1;
                } else {
                    // This shouldn't happen - indicates a bug
                    eprintln!("WARNING: Node48 remove_child couldn't find key_byte for idx {}", idx);
                }
                
                child
            }
            
            Node::Node256 { children, num_children, .. } => {
                *num_children -= 1;
                children[idx].take().expect("Child should exist at index")
            }
        }
    }

    /// Grow Node4 to Node16.
    fn grow_to_node16(&mut self, stats: &mut ArtMemoryStats) {
        if let Node::Node4 { prefix, keys, num_children, children, leaf_value, .. } = self {
            let mut new_keys = [0u8; 16];
            new_keys[..4].copy_from_slice(keys);
            
            *self = Node::Node16 {
                prefix: std::mem::take(prefix),
                num_children: *num_children,
                keys: new_keys,
                children: std::mem::take(children),
                leaf_value: std::mem::take(leaf_value),
            };
            
            stats.node4_count = stats.node4_count.saturating_sub(1);
            stats.node16_count += 1;
        }
    }

    /// Grow Node16 to Node48.
    fn grow_to_node48(&mut self, stats: &mut ArtMemoryStats) {
        if let Node::Node16 { prefix, keys, num_children, children, leaf_value, .. } = self {
            let mut child_index = Box::new([255u8; 256]);
            for i in 0..*num_children as usize {
                child_index[keys[i] as usize] = i as u8;
            }
            
            *self = Node::Node48 {
                prefix: std::mem::take(prefix),
                num_children: *num_children,
                child_index,
                children: std::mem::take(children),
                leaf_value: std::mem::take(leaf_value),
            };
            
            stats.node16_count = stats.node16_count.saturating_sub(1);
            stats.node48_count += 1;
        }
    }

    /// Grow Node48 to Node256.
    fn grow_to_node256(&mut self, stats: &mut ArtMemoryStats) {
        if let Node::Node48 { prefix, child_index, num_children, children, leaf_value, .. } = self {
            let mut new_children: Box<[Option<Box<Node<V>>>; 256]> = Box::new(std::array::from_fn(|_| None));
            
            for (byte, &idx) in child_index.iter().enumerate() {
                if idx != 255 && (idx as usize) < children.len() {
                    new_children[byte] = Some(std::mem::replace(
                        &mut children[idx as usize],
                        Box::new(Node::new_node4()),
                    ));
                }
            }
            
            *self = Node::Node256 {
                prefix: std::mem::take(prefix),
                num_children: *num_children as u16,
                children: new_children,
                leaf_value: std::mem::take(leaf_value),
            };
            
            stats.node48_count = stats.node48_count.saturating_sub(1);
            stats.node256_count += 1;
        }
    }
}

impl<V> std::fmt::Debug for Node<V> 
where
    V: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Node::Leaf { key, value } => {
                f.debug_struct("Leaf")
                    .field("key", &String::from_utf8_lossy(key))
                    .field("value", value)
                    .finish()
            }
            Node::Node4 { prefix, num_children, keys, .. } => {
                f.debug_struct("Node4")
                    .field("prefix", &String::from_utf8_lossy(prefix))
                    .field("num_children", num_children)
                    .field("keys", &keys[..*num_children as usize].iter().map(|k| *k as char).collect::<Vec<_>>())
                    .finish()
            }
            Node::Node16 { prefix, num_children, keys, .. } => {
                f.debug_struct("Node16")
                    .field("prefix", &String::from_utf8_lossy(prefix))
                    .field("num_children", num_children)
                    .field("keys", &keys[..*num_children as usize].iter().map(|k| *k as char).collect::<Vec<_>>())
                    .finish()
            }
            Node::Node48 { prefix, num_children, .. } => {
                f.debug_struct("Node48")
                    .field("prefix", &String::from_utf8_lossy(prefix))
                    .field("num_children", num_children)
                    .finish()
            }
            Node::Node256 { prefix, num_children, .. } => {
                f.debug_struct("Node256")
                    .field("prefix", &String::from_utf8_lossy(prefix))
                    .field("num_children", num_children)
                    .finish()
            }
        }
    }
}
