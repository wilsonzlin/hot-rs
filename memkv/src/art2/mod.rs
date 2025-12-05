//! Optimized ART implementation - stores keys implicitly in path.

use std::ops::{Bound, RangeBounds};

/// Memory statistics for the ART.
#[derive(Debug, Default, Clone)]
pub struct ArtMemoryStats {
    pub node4_count: usize,
    pub node16_count: usize,
    pub node48_count: usize,
    pub node256_count: usize,
    pub leaf_count: usize,
    pub node_bytes: usize,
    pub value_bytes: usize,
}

/// A node in the optimized ART.
/// Key insight: leaves store only the suffix (remaining key bytes after path).
pub enum Node<V> {
    /// A leaf node - stores suffix and value.
    Leaf {
        /// Remaining key bytes after the path to this node.
        suffix: Vec<u8>,
        /// The value.
        value: V,
    },
    /// Internal node with up to 4 children.
    Node4 {
        prefix: Vec<u8>,
        num_children: u8,
        keys: [u8; 4],
        children: Vec<Box<Node<V>>>,
        /// Value stored at this node (key ends here).
        value: Option<V>,
    },
    /// Internal node with 5-16 children.
    Node16 {
        prefix: Vec<u8>,
        num_children: u8,
        keys: [u8; 16],
        children: Vec<Box<Node<V>>>,
        value: Option<V>,
    },
    /// Internal node with 17-48 children.
    Node48 {
        prefix: Vec<u8>,
        num_children: u8,
        child_index: Box<[u8; 256]>,
        children: Vec<Box<Node<V>>>,
        value: Option<V>,
    },
    /// Internal node with 49-256 children.
    Node256 {
        prefix: Vec<u8>,
        num_children: u16,
        children: Box<[Option<Box<Node<V>>>; 256]>,
        value: Option<V>,
    },
}

impl<V> Node<V> {
    pub fn new_leaf(suffix: Vec<u8>, value: V) -> Self {
        Node::Leaf { suffix, value }
    }

    pub fn new_node4() -> Self {
        Node::Node4 {
            prefix: Vec::new(),
            num_children: 0,
            keys: [0; 4],
            children: Vec::with_capacity(4),
            value: None,
        }
    }
    
    pub fn new_node16() -> Self {
        Node::Node16 {
            prefix: Vec::new(),
            num_children: 0,
            keys: [0; 16],
            children: Vec::with_capacity(16),
            value: None,
        }
    }
    
    pub fn new_node48() -> Self {
        Node::Node48 {
            prefix: Vec::new(),
            num_children: 0,
            child_index: Box::new([255; 256]),
            children: Vec::with_capacity(48),
            value: None,
        }
    }
    
    pub fn new_node256() -> Self {
        Node::Node256 {
            prefix: Vec::new(),
            num_children: 0,
            children: Box::new(std::array::from_fn(|_| None)),
            value: None,
        }
    }

    pub fn prefix(&self) -> &[u8] {
        match self {
            Node::Leaf { .. } => &[],
            Node::Node4 { prefix, .. }
            | Node::Node16 { prefix, .. }
            | Node::Node48 { prefix, .. }
            | Node::Node256 { prefix, .. } => prefix,
        }
    }
    
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

    pub fn get_value(&self) -> Option<&V> {
        match self {
            Node::Leaf { value, .. } => Some(value),
            Node::Node4 { value, .. }
            | Node::Node16 { value, .. }
            | Node::Node48 { value, .. }
            | Node::Node256 { value, .. } => value.as_ref(),
        }
    }
    
    pub fn get_suffix(&self) -> &[u8] {
        match self {
            Node::Leaf { suffix, .. } => suffix,
            _ => &[],
        }
    }
    
    pub fn set_value(&mut self, new_value: Option<V>) {
        match self {
            Node::Leaf { value, .. } => {
                if let Some(v) = new_value {
                    *value = v;
                }
            }
            Node::Node4 { value, .. }
            | Node::Node16 { value, .. }
            | Node::Node48 { value, .. }
            | Node::Node256 { value, .. } => *value = new_value,
        }
    }
    
    pub fn take_value(&mut self) -> Option<V> {
        match self {
            Node::Leaf { .. } => None, // Can't take from leaf
            Node::Node4 { value, .. }
            | Node::Node16 { value, .. }
            | Node::Node48 { value, .. }
            | Node::Node256 { value, .. } => value.take(),
        }
    }

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

    pub fn add_child(&mut self, key: u8, child: Box<Node<V>>) {
        match self {
            Node::Leaf { .. } => panic!("Cannot add child to leaf"),
            Node::Node4 { keys, num_children, children, .. } => {
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
                    panic!("Node4 is full");
                }
            }
            Node::Node16 { keys, num_children, children, .. } => {
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
            Node::Node48 { child_index, num_children, children, .. } => {
                let existing_idx = child_index[key as usize];
                if existing_idx != 255 && (existing_idx as usize) < children.len() {
                    children[existing_idx as usize] = child;
                } else if (*num_children as usize) < 48 {
                    let slot = children.len();
                    if slot < 48 {
                        child_index[key as usize] = slot as u8;
                        children.push(child);
                        *num_children += 1;
                    }
                } else {
                    panic!("Node48 is full");
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

    pub fn add_child_grow(&mut self, key: u8, child: Box<Node<V>>, stats: &mut ArtMemoryStats) {
        match self {
            Node::Node4 { num_children, .. } if *num_children >= 4 => {
                self.grow_to_node16(stats);
                self.add_child(key, child);
            }
            Node::Node16 { num_children, .. } if *num_children >= 16 => {
                self.grow_to_node48(stats);
                self.add_child(key, child);
            }
            Node::Node48 { num_children, .. } if *num_children >= 48 => {
                self.grow_to_node256(stats);
                self.add_child(key, child);
            }
            _ => self.add_child(key, child),
        }
    }

    pub fn remove_child(&mut self, idx: usize) -> Box<Node<V>> {
        match self {
            Node::Leaf { .. } => panic!("Cannot remove from leaf"),
            Node::Node4 { keys, num_children, children, .. } => {
                let child = children.remove(idx);
                keys.copy_within((idx + 1)..*num_children as usize, idx);
                *num_children -= 1;
                child
            }
            Node::Node16 { keys, num_children, children, .. } => {
                let child = children.remove(idx);
                keys.copy_within((idx + 1)..*num_children as usize, idx);
                *num_children -= 1;
                child
            }
            Node::Node48 { child_index, num_children, children, .. } => {
                let mut key_byte = None;
                for (k, &i) in child_index.iter().enumerate() {
                    if i != 255 && i as usize == idx {
                        key_byte = Some(k);
                        break;
                    }
                }
                let child = std::mem::replace(&mut children[idx], Box::new(Node::new_node4()));
                if let Some(kb) = key_byte {
                    child_index[kb] = 255;
                    *num_children -= 1;
                }
                child
            }
            Node::Node256 { children, num_children, .. } => {
                *num_children -= 1;
                children[idx].take().expect("Child should exist")
            }
        }
    }

    fn grow_to_node16(&mut self, stats: &mut ArtMemoryStats) {
        if let Node::Node4 { prefix, keys, num_children, children, value, .. } = self {
            let mut new_keys = [0u8; 16];
            new_keys[..4].copy_from_slice(keys);
            *self = Node::Node16 {
                prefix: std::mem::take(prefix),
                num_children: *num_children,
                keys: new_keys,
                children: std::mem::take(children),
                value: std::mem::take(value),
            };
            stats.node4_count = stats.node4_count.saturating_sub(1);
            stats.node16_count += 1;
        }
    }

    fn grow_to_node48(&mut self, stats: &mut ArtMemoryStats) {
        if let Node::Node16 { prefix, keys, num_children, children, value, .. } = self {
            let mut child_index = Box::new([255u8; 256]);
            for i in 0..*num_children as usize {
                child_index[keys[i] as usize] = i as u8;
            }
            *self = Node::Node48 {
                prefix: std::mem::take(prefix),
                num_children: *num_children,
                child_index,
                children: std::mem::take(children),
                value: std::mem::take(value),
            };
            stats.node16_count = stats.node16_count.saturating_sub(1);
            stats.node48_count += 1;
        }
    }

    fn grow_to_node256(&mut self, stats: &mut ArtMemoryStats) {
        if let Node::Node48 { prefix, child_index, num_children, children, value, .. } = self {
            let mut new_children: Box<[Option<Box<Node<V>>>; 256]> = 
                Box::new(std::array::from_fn(|_| None));
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
                value: std::mem::take(value),
            };
            stats.node48_count = stats.node48_count.saturating_sub(1);
            stats.node256_count += 1;
        }
    }
}

/// Optimized Adaptive Radix Tree.
pub struct OptimizedART<V> {
    root: Option<Box<Node<V>>>,
    size: usize,
    stats: ArtMemoryStats,
}

impl<V: Clone> Default for OptimizedART<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> OptimizedART<V> {
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
            stats: ArtMemoryStats::default(),
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        if self.root.is_none() {
            // Store full key as suffix since path is empty
            self.root = Some(Box::new(Node::new_leaf(key.to_vec(), value)));
            self.size = 1;
            self.stats.leaf_count = 1;
            return None;
        }

        let root = self.root.take().unwrap();
        let (new_root, old_value) = self.insert_recursive(root, key, 0, value);
        self.root = Some(new_root);
        if old_value.is_none() {
            self.size += 1;
        }
        old_value
    }

    fn insert_recursive(
        &mut self,
        mut node: Box<Node<V>>,
        key: &[u8],
        depth: usize,
        value: V,
    ) -> (Box<Node<V>>, Option<V>) {
        match &*node {
            Node::Leaf { suffix, .. } => {
                let suffix = suffix.clone();
                let remaining_key = &key[depth..];
                
                // Find common prefix between suffix and remaining key
                let common_len = suffix.iter()
                    .zip(remaining_key.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                if common_len == suffix.len() && common_len == remaining_key.len() {
                    // Same key - replace value
                    if let Node::Leaf { value: old_val, .. } = &mut *node {
                        let old = std::mem::replace(old_val, value);
                        return (node, Some(old));
                    }
                }
                
                // Need to split
                let mut new_inner = Box::new(Node::new_node4());
                
                // Set prefix to common part
                if common_len > 0 {
                    new_inner.set_prefix(&suffix[..common_len]);
                }
                
                // Where do old and new diverge?
                if common_len < suffix.len() {
                    // Old leaf has more bytes - add as child
                    let old_byte = suffix[common_len];
                    if let Node::Leaf { value: old_val, .. } = *node {
                        let old_leaf = Box::new(Node::new_leaf(
                            suffix[common_len + 1..].to_vec(),
                            old_val
                        ));
                        new_inner.add_child(old_byte, old_leaf);
                    }
                } else {
                    // Old key ends at the split point - store value at node
                    if let Node::Leaf { value: old_val, .. } = *node {
                        new_inner.set_value(Some(old_val));
                    }
                }
                
                if common_len < remaining_key.len() {
                    // New key has more bytes - add as child
                    let new_byte = remaining_key[common_len];
                    let new_leaf = Box::new(Node::new_leaf(
                        remaining_key[common_len + 1..].to_vec(),
                        value
                    ));
                    new_inner.add_child(new_byte, new_leaf);
                    self.stats.leaf_count += 1;
                } else {
                    // New key ends at split point
                    new_inner.set_value(Some(value));
                }
                
                self.stats.node4_count += 1;
                return (new_inner, None);
            }
            _ => {}
        }

        let prefix = node.prefix().to_vec();
        let prefix_len = prefix.len();
        
        // Check prefix match
        let prefix_match = prefix
            .iter()
            .zip(key[depth..].iter())
            .take_while(|(a, b)| a == b)
            .count();

        if prefix_match < prefix_len {
            // Prefix mismatch - need to split
            let mut new_inner = Box::new(Node::new_node4());
            new_inner.set_prefix(&prefix[..prefix_match]);
            
            // Old node keeps remaining prefix
            let old_prefix_byte = prefix[prefix_match];
            node.set_prefix(&prefix[prefix_match + 1..]);
            new_inner.add_child(old_prefix_byte, node);
            
            // Add new key
            let new_key_depth = depth + prefix_match;
            if new_key_depth < key.len() {
                let new_byte = key[new_key_depth];
                let suffix = key[new_key_depth + 1..].to_vec();
                let new_leaf = Box::new(Node::new_leaf(suffix, value));
                new_inner.add_child(new_byte, new_leaf);
                self.stats.leaf_count += 1;
            } else {
                new_inner.set_value(Some(value));
            }
            
            self.stats.node4_count += 1;
            return (new_inner, None);
        }

        // Full prefix match
        let next_depth = depth + prefix_len;

        if next_depth >= key.len() {
            // Key ends at this node
            let old = node.take_value();
            node.set_value(Some(value));
            if old.is_none() {
                self.stats.leaf_count += 1;
            }
            return (node, old);
        }

        let next_byte = key[next_depth];

        if let Some(child_idx) = node.find_child(next_byte) {
            let child = node.remove_child(child_idx);
            let (new_child, old_value) = self.insert_recursive(child, key, next_depth + 1, value);
            node.add_child(next_byte, new_child);
            (node, old_value)
        } else {
            let suffix = key[next_depth + 1..].to_vec();
            let new_leaf = Box::new(Node::new_leaf(suffix, value));
            node.add_child_grow(next_byte, new_leaf, &mut self.stats);
            self.stats.leaf_count += 1;
            (node, None)
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<&V> {
        let root = self.root.as_ref()?;
        self.get_recursive(root, key, 0)
    }

    fn get_recursive<'a>(&'a self, node: &'a Node<V>, key: &[u8], depth: usize) -> Option<&'a V> {
        match node {
            Node::Leaf { suffix, value } => {
                // Check if remaining key matches suffix
                let remaining = &key[depth..];
                if remaining == suffix.as_slice() {
                    Some(value)
                } else {
                    None
                }
            }
            _ => {
                let prefix = node.prefix();
                let prefix_len = prefix.len();
                
                // Check prefix
                if key.len() < depth + prefix_len {
                    return None;
                }
                for (i, &b) in prefix.iter().enumerate() {
                    if key[depth + i] != b {
                        return None;
                    }
                }
                
                let next_depth = depth + prefix_len;
                if next_depth >= key.len() {
                    return node.get_value();
                }

                let next_byte = key[next_depth];
                if let Some(child_idx) = node.find_child(next_byte) {
                    match node {
                        Node::Node4 { children, .. }
                        | Node::Node16 { children, .. }
                        | Node::Node48 { children, .. } => {
                            self.get_recursive(&children[child_idx], key, next_depth + 1)
                        }
                        Node::Node256 { children, .. } => {
                            if let Some(ref child) = children[child_idx] {
                                self.get_recursive(child, key, next_depth + 1)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub fn memory_stats(&self) -> ArtMemoryStats {
        let mut stats = ArtMemoryStats::default();
        if let Some(ref root) = self.root {
            self.compute_stats(root, &mut stats);
        }
        stats
    }

    fn compute_stats(&self, node: &Node<V>, stats: &mut ArtMemoryStats) {
        match node {
            Node::Leaf { suffix, .. } => {
                stats.leaf_count += 1;
                stats.value_bytes += std::mem::size_of::<V>();
                // suffix Vec (24) + suffix data + value
                stats.node_bytes += 24 + suffix.capacity() + std::mem::size_of::<V>();
            }
            Node::Node4 { prefix, children, value, num_children, .. } => {
                stats.node4_count += 1;
                stats.node_bytes += 24 + prefix.capacity() + 4 + 24 + (*num_children as usize) * 8;
                if value.is_some() {
                    stats.value_bytes += std::mem::size_of::<V>();
                }
                for child in children.iter() {
                    self.compute_stats(child, stats);
                }
            }
            Node::Node16 { prefix, children, value, num_children, .. } => {
                stats.node16_count += 1;
                stats.node_bytes += 24 + prefix.capacity() + 16 + 24 + (*num_children as usize) * 8;
                if value.is_some() {
                    stats.value_bytes += std::mem::size_of::<V>();
                }
                for child in children.iter() {
                    self.compute_stats(child, stats);
                }
            }
            Node::Node48 { prefix, children, value, num_children, .. } => {
                stats.node48_count += 1;
                stats.node_bytes += 24 + prefix.capacity() + 256 + 24 + (*num_children as usize) * 8;
                if value.is_some() {
                    stats.value_bytes += std::mem::size_of::<V>();
                }
                for child in children.iter() {
                    self.compute_stats(child, stats);
                }
            }
            Node::Node256 { prefix, children, value, .. } => {
                stats.node256_count += 1;
                stats.node_bytes += 24 + prefix.capacity() + 256 * 8;
                if value.is_some() {
                    stats.value_bytes += std::mem::size_of::<V>();
                }
                for child in children.iter().flatten() {
                    self.compute_stats(child, stats);
                }
            }
        }
    }

    /// Debug print the tree structure.
    #[allow(dead_code)]
    pub fn debug_print(&self) where V: std::fmt::Debug {
        if let Some(ref root) = self.root {
            Self::print_node(root, 0, &[]);
        } else {
            eprintln!("(empty tree)");
        }
    }
    
    #[allow(dead_code)]
    fn print_node(node: &Node<V>, indent: usize, path: &[u8]) where V: std::fmt::Debug {
        let pad = "  ".repeat(indent);
        match node {
            Node::Leaf { suffix, value } => {
                let full_key: Vec<u8> = path.iter().chain(suffix.iter()).cloned().collect();
                eprintln!("{}Leaf: {:?} -> {:?}", pad, 
                    String::from_utf8_lossy(&full_key), value);
            }
            _ => {
                let prefix = node.prefix();
                let new_path: Vec<u8> = path.iter().chain(prefix.iter()).cloned().collect();
                eprintln!("{}Node (prefix={:?}):", pad, 
                    String::from_utf8_lossy(prefix));
                
                if let Some(v) = node.get_value() {
                    eprintln!("{}  [value at this node: {:?}]", pad, v);
                }
                
                match node {
                    Node::Node4 { keys, num_children, children, .. } => {
                        for i in 0..*num_children as usize {
                            let key_byte = keys[i];
                            let mut child_path = new_path.clone();
                            child_path.push(key_byte);
                            eprintln!("{}  [{} / 0x{:02x}] ->", pad, key_byte as char, key_byte);
                            Self::print_node(&children[i], indent + 2, &child_path);
                        }
                    }
                    Node::Node16 { keys, num_children, children, .. } => {
                        for i in 0..*num_children as usize {
                            let key_byte = keys[i];
                            let mut child_path = new_path.clone();
                            child_path.push(key_byte);
                            eprintln!("{}  [{} / 0x{:02x}] ->", pad, key_byte as char, key_byte);
                            Self::print_node(&children[i], indent + 2, &child_path);
                        }
                    }
                    Node::Node48 { child_index, children, .. } => {
                        for byte in 0..=255u8 {
                            let idx = child_index[byte as usize];
                            if idx != 255 && (idx as usize) < children.len() {
                                let mut child_path = new_path.clone();
                                child_path.push(byte);
                                eprintln!("{}  [{} / 0x{:02x}] ->", pad, byte as char, byte);
                                Self::print_node(&children[idx as usize], indent + 2, &child_path);
                            }
                        }
                    }
                    Node::Node256 { children, .. } => {
                        for byte in 0u8..=255 {
                            if let Some(ref child) = children[byte as usize] {
                                let mut child_path = new_path.clone();
                                child_path.push(byte);
                                eprintln!("{}  [{} / 0x{:02x}] ->", pad, byte as char, byte);
                                Self::print_node(child, indent + 2, &child_path);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Iterate over all key-value pairs, reconstructing keys from paths.
    pub fn iter(&self) -> impl Iterator<Item = (Vec<u8>, &V)> {
        let mut results = Vec::new();
        if let Some(ref root) = self.root {
            self.collect_all(root, Vec::new(), &mut results);
        }
        results.into_iter()
    }

    fn collect_all<'a>(&'a self, node: &'a Node<V>, mut path: Vec<u8>, results: &mut Vec<(Vec<u8>, &'a V)>) {
        match node {
            Node::Leaf { suffix, value } => {
                path.extend_from_slice(suffix);
                results.push((path, value));
            }
            _ => {
                // Add prefix to path
                path.extend_from_slice(node.prefix());
                
                // Check for value at this node
                if let Some(value) = node.get_value() {
                    results.push((path.clone(), value));
                }

                // Visit children
                match node {
                    Node::Node4 { keys, num_children, children, .. } => {
                        for i in 0..*num_children as usize {
                            let mut child_path = path.clone();
                            child_path.push(keys[i]);
                            self.collect_all(&children[i], child_path, results);
                        }
                    }
                    Node::Node16 { keys, num_children, children, .. } => {
                        for i in 0..*num_children as usize {
                            let mut child_path = path.clone();
                            child_path.push(keys[i]);
                            self.collect_all(&children[i], child_path, results);
                        }
                    }
                    Node::Node48 { child_index, children, .. } => {
                        for byte in 0..=255u8 {
                            let idx = child_index[byte as usize];
                            if idx != 255 && (idx as usize) < children.len() {
                                let mut child_path = path.clone();
                                child_path.push(byte);
                                self.collect_all(&children[idx as usize], child_path, results);
                            }
                        }
                    }
                    Node::Node256 { children, .. } => {
                        for (byte, child_opt) in children.iter().enumerate() {
                            if let Some(ref child) = child_opt {
                                let mut child_path = path.clone();
                                child_path.push(byte as u8);
                                self.collect_all(child, child_path, results);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let mut tree = OptimizedART::new();
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        tree.insert(b"help", 3);
        
        assert_eq!(tree.get(b"hello"), Some(&1));
        assert_eq!(tree.get(b"world"), Some(&2));
        assert_eq!(tree.get(b"help"), Some(&3));
        assert_eq!(tree.get(b"hell"), None);
        
        assert_eq!(tree.len(), 3);
    }

    #[test]
    fn test_iter() {
        let mut tree = OptimizedART::new();
        tree.insert(b"a", 1);
        tree.insert(b"ab", 2);
        tree.insert(b"abc", 3);
        
        let items: Vec<_> = tree.iter().collect();
        assert_eq!(items.len(), 3);
    }
    
    #[test]
    fn test_prefix_sharing() {
        let mut tree = OptimizedART::new();
        tree.insert(b"http://example.com/page1", 1);
        tree.insert(b"http://example.com/page2", 2);
        tree.insert(b"http://example.com/page3", 3);
        tree.insert(b"http://other.com/page1", 4);
        
        assert_eq!(tree.get(b"http://example.com/page1"), Some(&1));
        assert_eq!(tree.get(b"http://example.com/page2"), Some(&2));
        assert_eq!(tree.get(b"http://example.com/page3"), Some(&3));
        assert_eq!(tree.get(b"http://other.com/page1"), Some(&4));
        
        assert_eq!(tree.len(), 4);
    }
    
    #[test]
    fn test_update() {
        let mut tree = OptimizedART::new();
        assert_eq!(tree.insert(b"key", 1), None);
        assert_eq!(tree.insert(b"key", 2), Some(1));
        assert_eq!(tree.get(b"key"), Some(&2));
        assert_eq!(tree.len(), 1);
    }
    
    #[test]
    fn test_many_keys() {
        let mut tree = OptimizedART::new();
        for i in 0..1000 {
            let key = format!("key{:04}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        assert_eq!(tree.len(), 1000);
        
        for i in 0..1000 {
            let key = format!("key{:04}", i);
            assert_eq!(tree.get(key.as_bytes()), Some(&i), "Failed for key {}", key);
        }
    }
}
