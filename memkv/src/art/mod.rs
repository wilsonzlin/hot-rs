//! Adaptive Radix Tree (ART) implementation.
//!
//! Based on "The Adaptive Radix Tree: ARTful Indexing for Main-Memory Databases"
//! by Leis et al., 2013.
//!
//! Key features:
//! - Adaptive node sizes (4, 16, 48, 256 children)
//! - Path compression for common prefixes
//! - Optimized for modern CPU caches

mod node;
mod debug;

use std::ops::{Bound, RangeBounds};

pub use node::{Node, NodeType};

/// Memory statistics for the ART.
#[derive(Debug, Clone, Default)]
pub struct ArtMemoryStats {
    /// Bytes used for key storage
    pub key_bytes: usize,
    /// Bytes used for node structures
    pub node_bytes: usize,
    /// Bytes used for value storage
    pub value_bytes: usize,
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
}

/// An Adaptive Radix Tree for efficient key-value storage.
pub struct AdaptiveRadixTree<V> {
    root: Option<Box<Node<V>>>,
    size: usize,
    stats: ArtMemoryStats,
}

impl<V> AdaptiveRadixTree<V> {
    /// Create a new empty ART.
    pub fn new() -> Self {
        Self {
            root: None,
            size: 0,
            stats: ArtMemoryStats::default(),
        }
    }

    /// Insert a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V>
    where
        V: Clone,
    {
        if self.root.is_none() {
            // Tree is empty, create a leaf
            self.root = Some(Box::new(Node::new_leaf(key.to_vec(), value)));
            self.size = 1;
            self.stats.leaf_count = 1;
            self.stats.key_bytes += key.len();
            return None;
        }

        let root = self.root.take().unwrap();
        let (new_root, old_value) = Self::insert_recursive(root, key, 0, value, &mut self.stats);
        self.root = Some(new_root);
        
        if old_value.is_none() {
            self.size += 1;
        }
        old_value
    }

    fn insert_recursive(
        mut node: Box<Node<V>>,
        key: &[u8],
        depth: usize,
        value: V,
        stats: &mut ArtMemoryStats,
    ) -> (Box<Node<V>>, Option<V>)
    where
        V: Clone,
    {
        match &mut *node {
            Node::Leaf { key: leaf_key, value: leaf_value } => {
                // Check if keys are equal
                if leaf_key.as_slice() == key {
                    // Replace value
                    let old = std::mem::replace(leaf_value, value);
                    return (node, Some(old));
                }

                // Keys differ, need to split
                // Find the common prefix length
                let common_prefix_len = leaf_key[depth..]
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();

                let split_depth = depth + common_prefix_len;
                
                // Create a new inner node at the split point
                let mut new_inner = Box::new(Node::new_node4());
                
                // Determine the branching bytes
                let existing_byte = leaf_key.get(split_depth).copied();
                let new_byte = key.get(split_depth).copied();

                // Handle the case where one key is a prefix of the other
                match (existing_byte, new_byte) {
                    (Some(eb), Some(nb)) => {
                        let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                        new_inner.add_child(eb, node);
                        new_inner.add_child(nb, new_leaf);
                        stats.leaf_count += 1;
                        stats.key_bytes += key.len();
                    }
                    (Some(eb), None) => {
                        // New key is prefix of existing key
                        // The new key becomes a value at this node
                        // For simplicity, we'll create a special leaf for the prefix
                        let prefix_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                        // Use 0 as a special marker for "end of key" (this is a simplification)
                        new_inner.add_child(eb, node);
                        new_inner.set_leaf_value(Some((key.to_vec(), prefix_leaf)));
                        stats.leaf_count += 1;
                        stats.key_bytes += key.len();
                    }
                    (None, Some(nb)) => {
                        // Existing key is prefix of new key
                        let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                        new_inner.add_child(nb, new_leaf);
                        new_inner.set_leaf_value(Some((leaf_key.clone(), node)));
                        stats.leaf_count += 1;
                        stats.key_bytes += key.len();
                    }
                    (None, None) => {
                        // Keys are equal (should have been caught above)
                        unreachable!("Keys should have been equal");
                    }
                }

                // Store the prefix
                if common_prefix_len > 0 {
                    new_inner.set_prefix(&key[depth..split_depth]);
                }

                stats.node4_count += 1;
                (new_inner, None)
            }

            Node::Node4 { .. }
            | Node::Node16 { .. }
            | Node::Node48 { .. }
            | Node::Node256 { .. } => {
                let prefix = node.prefix().to_vec();
                
                // Check prefix match
                let prefix_len = prefix.len();
                let prefix_match = prefix
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();

                if prefix_match < prefix_len {
                    // Prefix mismatch - need to split the node
                    let mut new_inner = Box::new(Node::new_node4());
                    
                    // The new node's prefix is the common part
                    new_inner.set_prefix(&prefix[..prefix_match]);
                    
                    // The old node keeps the rest of its prefix
                    let old_prefix_byte = prefix[prefix_match];
                    let old_remaining_prefix = prefix[prefix_match + 1..].to_vec();
                    node.set_prefix(&old_remaining_prefix);
                    
                    // Add old node as child
                    new_inner.add_child(old_prefix_byte, node);
                    
                    // Add new leaf for the new key
                    let new_key_depth = depth + prefix_match;
                    if new_key_depth < key.len() {
                        let new_byte = key[new_key_depth];
                        let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                        new_inner.add_child(new_byte, new_leaf);
                        stats.leaf_count += 1;
                        stats.key_bytes += key.len();
                    } else {
                        // New key ends at the split point
                        let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                        new_inner.set_leaf_value(Some((key.to_vec(), new_leaf)));
                        stats.leaf_count += 1;
                        stats.key_bytes += key.len();
                    }
                    
                    stats.node4_count += 1;
                    return (new_inner, None);
                }

                // Full prefix match
                let next_depth = depth + prefix_len;

                if next_depth >= key.len() {
                    // Key ends at this node
                    if let Some((_, ref existing_leaf)) = node.get_leaf_value() {
                        if let Node::Leaf { value: ref v, .. } = **existing_leaf {
                            // Need to update - set leaf value with new value
                            let old_val = v.clone();
                            let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                            node.set_leaf_value(Some((key.to_vec(), new_leaf)));
                            return (node, Some(old_val));
                        }
                    }
                    // No existing value at this node
                    let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                    node.set_leaf_value(Some((key.to_vec(), new_leaf)));
                    stats.leaf_count += 1;
                    stats.key_bytes += key.len();
                    return (node, None);
                }

                let next_byte = key[next_depth];
                
                // Look for child with this byte
                if let Some(child_idx) = node.find_child(next_byte) {
                    // Recurse into existing child
                    let child = node.remove_child(child_idx);
                    let (new_child, old_value) = Self::insert_recursive(child, key, next_depth + 1, value, stats);
                    node.add_child(next_byte, new_child);
                    (node, old_value)
                } else {
                    // No child with this byte, add new leaf
                    let new_leaf = Box::new(Node::new_leaf(key.to_vec(), value));
                    node.add_child_grow(next_byte, new_leaf, stats);
                    stats.leaf_count += 1;
                    stats.key_bytes += key.len();
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
                Node::Leaf { key: leaf_key, value } => {
                    if leaf_key.as_slice() == key {
                        return Some(value);
                    } else {
                        return None;
                    }
                }

                Node::Node4 { prefix, children, leaf_value, .. } => {
                    // Check prefix
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;

                    if depth >= key.len() {
                        // Key ends at this node
                        if let Some((_, ref leaf)) = leaf_value {
                            if let Node::Leaf { key: ref lk, ref value } = **leaf {
                                if lk.as_slice() == key {
                                    return Some(value);
                                }
                            }
                        }
                        return None;
                    }

                    let next_byte = key[depth];
                    if let Some(child_idx) = Self::find_child_in_node(node, next_byte) {
                        node = &children[child_idx];
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                Node::Node16 { prefix, children, leaf_value, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;

                    if depth >= key.len() {
                        if let Some((_, ref leaf)) = leaf_value {
                            if let Node::Leaf { key: ref lk, ref value } = **leaf {
                                if lk.as_slice() == key {
                                    return Some(value);
                                }
                            }
                        }
                        return None;
                    }

                    let next_byte = key[depth];
                    if let Some(child_idx) = Self::find_child_in_node(node, next_byte) {
                        node = &children[child_idx];
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                Node::Node48 { prefix, children, leaf_value, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;

                    if depth >= key.len() {
                        if let Some((_, ref leaf)) = leaf_value {
                            if let Node::Leaf { key: ref lk, ref value } = **leaf {
                                if lk.as_slice() == key {
                                    return Some(value);
                                }
                            }
                        }
                        return None;
                    }

                    let next_byte = key[depth];
                    if let Some(child_idx) = Self::find_child_in_node(node, next_byte) {
                        node = &children[child_idx];
                        depth += 1;
                    } else {
                        return None;
                    }
                }
                
                Node::Node256 { prefix, children, leaf_value, .. } => {
                    let prefix_len = prefix.len();
                    if key.len() < depth + prefix_len {
                        return None;
                    }
                    if &key[depth..depth + prefix_len] != prefix.as_slice() {
                        return None;
                    }
                    depth += prefix_len;

                    if depth >= key.len() {
                        if let Some((_, ref leaf)) = leaf_value {
                            if let Node::Leaf { key: ref lk, ref value } = **leaf {
                                if lk.as_slice() == key {
                                    return Some(value);
                                }
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

    fn find_child_in_node(node: &Node<V>, byte: u8) -> Option<usize> {
        match node {
            Node::Node4 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        return Some(i);
                    }
                }
                None
            }
            Node::Node16 { keys, num_children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == byte {
                        return Some(i);
                    }
                }
                None
            }
            Node::Node48 { child_index, .. } => {
                let idx = child_index[byte as usize];
                if idx != 255 {
                    Some(idx as usize)
                } else {
                    None
                }
            }
            Node::Node256 { .. } => {
                Some(byte as usize)
            }
            Node::Leaf { .. } => None,
        }
    }

    /// Remove a key from the tree.
    pub fn remove(&mut self, key: &[u8]) -> Option<V>
    where
        V: Clone,
    {
        if self.root.is_none() {
            return None;
        }

        let root = self.root.take().unwrap();
        let (new_root, old_value) = Self::remove_recursive(root, key, 0, &mut self.stats);
        self.root = new_root;
        
        if old_value.is_some() {
            self.size -= 1;
        }
        old_value
    }

    fn remove_recursive(
        mut node: Box<Node<V>>,
        key: &[u8],
        depth: usize,
        stats: &mut ArtMemoryStats,
    ) -> (Option<Box<Node<V>>>, Option<V>)
    where
        V: Clone,
    {
        match &mut *node {
            Node::Leaf { key: leaf_key, value } => {
                if leaf_key.as_slice() == key {
                    stats.leaf_count = stats.leaf_count.saturating_sub(1);
                    stats.key_bytes = stats.key_bytes.saturating_sub(leaf_key.len());
                    (None, Some(value.clone()))
                } else {
                    (Some(node), None)
                }
            }

            Node::Node4 { .. }
            | Node::Node16 { .. }
            | Node::Node48 { .. }
            | Node::Node256 { .. } => {
                let prefix = node.prefix().to_vec();
                let prefix_len = prefix.len();
                
                // Check prefix match
                if key.len() < depth + prefix_len {
                    return (Some(node), None);
                }
                if &key[depth..depth + prefix_len] != prefix.as_slice() {
                    return (Some(node), None);
                }

                let next_depth = depth + prefix_len;

                if next_depth >= key.len() {
                    // Key ends at this node - check leaf value
                    let should_remove = if let Some((ref lk, _)) = node.get_leaf_value() {
                        lk.as_slice() == key
                    } else {
                        false
                    };
                    
                    if should_remove {
                        if let Some((k, old_leaf)) = node.take_leaf_value() {
                            if let Node::Leaf { key: ref leaf_k, ref value } = *old_leaf {
                                stats.leaf_count = stats.leaf_count.saturating_sub(1);
                                stats.key_bytes = stats.key_bytes.saturating_sub(leaf_k.len());
                                let _ = k; // Suppress warning
                                return (Some(node), Some(value.clone()));
                            }
                        }
                    }
                    return (Some(node), None);
                }

                let next_byte = key[next_depth];
                
                if let Some(child_idx) = node.find_child(next_byte) {
                    let child = node.remove_child(child_idx);
                    let (new_child, old_value) = Self::remove_recursive(child, key, next_depth + 1, stats);
                    
                    if let Some(c) = new_child {
                        node.add_child(next_byte, c);
                    }
                    
                    // If node is now empty, return None
                    if node.num_children() == 0 && node.get_leaf_value().is_none() {
                        // Decrement node count
                        match &*node {
                            Node::Node4 { .. } => stats.node4_count = stats.node4_count.saturating_sub(1),
                            Node::Node16 { .. } => stats.node16_count = stats.node16_count.saturating_sub(1),
                            Node::Node48 { .. } => stats.node48_count = stats.node48_count.saturating_sub(1),
                            Node::Node256 { .. } => stats.node256_count = stats.node256_count.saturating_sub(1),
                            _ => {}
                        }
                        return (None, old_value);
                    }
                    
                    (Some(node), old_value)
                } else {
                    (Some(node), None)
                }
            }
        }
    }

    /// Get the number of keys in the tree.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Check if the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Get memory statistics.
    pub fn memory_stats(&self) -> ArtMemoryStats {
        // Recompute stats by walking the tree
        let mut stats = ArtMemoryStats::default();
        if let Some(ref root) = self.root {
            Self::compute_stats(root, &mut stats);
        }
        stats
    }
    
    fn compute_stats(node: &Node<V>, stats: &mut ArtMemoryStats) {
        match node {
            Node::Leaf { key, .. } => {
                stats.leaf_count += 1;
                stats.key_bytes += key.len();
                stats.value_bytes += std::mem::size_of::<V>();
                // Leaf node size: key Vec (24) + key data + value
                stats.node_bytes += 24 + key.capacity() + std::mem::size_of::<V>();
            }
            Node::Node4 { prefix, children, leaf_value, num_children, .. } => {
                stats.node4_count += 1;
                // Node4 size: prefix Vec (24) + prefix data + keys[4] + children Vec (24) + pointers
                stats.node_bytes += 24 + prefix.capacity() + 4 + 24 + (*num_children as usize) * 8;
                stats.node_bytes += std::mem::size_of::<u8>() + 24; // num_children + leaf_value Option
                
                if let Some((key, leaf)) = leaf_value {
                    stats.key_bytes += key.len();
                    Self::compute_stats(leaf, stats);
                }
                
                for child in children.iter() {
                    Self::compute_stats(child, stats);
                }
            }
            Node::Node16 { prefix, children, leaf_value, num_children, .. } => {
                stats.node16_count += 1;
                stats.node_bytes += 24 + prefix.capacity() + 16 + 24 + (*num_children as usize) * 8;
                stats.node_bytes += std::mem::size_of::<u8>() + 24;
                
                if let Some((key, leaf)) = leaf_value {
                    stats.key_bytes += key.len();
                    Self::compute_stats(leaf, stats);
                }
                
                for child in children.iter() {
                    Self::compute_stats(child, stats);
                }
            }
            Node::Node48 { prefix, children, leaf_value, num_children, .. } => {
                stats.node48_count += 1;
                stats.node_bytes += 24 + prefix.capacity() + 256 + 24 + (*num_children as usize) * 8;
                stats.node_bytes += std::mem::size_of::<u8>() + 24;
                
                if let Some((key, leaf)) = leaf_value {
                    stats.key_bytes += key.len();
                    Self::compute_stats(leaf, stats);
                }
                
                for child in children.iter() {
                    Self::compute_stats(child, stats);
                }
            }
            Node::Node256 { prefix, children, leaf_value, .. } => {
                stats.node256_count += 1;
                // Fixed array of 256 Option<Box<Node>>
                stats.node_bytes += 24 + prefix.capacity() + 256 * 8;
                stats.node_bytes += std::mem::size_of::<u16>() + 24;
                
                if let Some((key, leaf)) = leaf_value {
                    stats.key_bytes += key.len();
                    Self::compute_stats(leaf, stats);
                }
                
                for child in children.iter().flatten() {
                    Self::compute_stats(child, stats);
                }
            }
        }
    }

    /// Iterate over a range of keys.
    pub fn range<'a, R>(&'a self, range: R) -> impl Iterator<Item = (Vec<u8>, V)> + 'a
    where
        R: RangeBounds<&'a [u8]>,
        V: Clone,
    {
        let start = match range.start_bound() {
            Bound::Included(s) => Bound::Included(s.to_vec()),
            Bound::Excluded(s) => Bound::Excluded(s.to_vec()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let end = match range.end_bound() {
            Bound::Included(e) => Bound::Included(e.to_vec()),
            Bound::Excluded(e) => Bound::Excluded(e.to_vec()),
            Bound::Unbounded => Bound::Unbounded,
        };

        RangeIterator::new(self, start, end)
    }

    /// Iterate over all keys with a given prefix.
    pub fn prefix_scan<'a>(&'a self, prefix: &[u8]) -> impl Iterator<Item = (Vec<u8>, V)> + 'a
    where
        V: Clone,
    {
        // A prefix scan is a range from [prefix, prefix + 1)
        // where prefix + 1 is the next prefix after all keys starting with prefix
        let start = prefix.to_vec();
        let end = Self::prefix_end(prefix);
        
        RangeIterator::new(
            self,
            Bound::Included(start),
            match end {
                Some(e) => Bound::Excluded(e),
                None => Bound::Unbounded,
            },
        )
    }

    /// Compute the exclusive end key for a prefix scan.
    fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
        let mut end = prefix.to_vec();
        // Increment the last byte, with carry
        while let Some(last) = end.pop() {
            if last < 255 {
                end.push(last + 1);
                return Some(end);
            }
            // Carry - continue to next byte
        }
        // All bytes were 255 - no upper bound
        None
    }
}

impl<V> Default for AdaptiveRadixTree<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over a range of keys.
/// Uses a simple recursive collection approach for correctness.
struct RangeIterator<V> {
    results: std::vec::IntoIter<(Vec<u8>, V)>,
}

impl<V: Clone> RangeIterator<V> {
    fn new(tree: &AdaptiveRadixTree<V>, start: Bound<Vec<u8>>, end: Bound<Vec<u8>>) -> Self {
        let mut results = Vec::new();
        
        if let Some(ref root) = tree.root {
            Self::collect_range(root, &start, &end, &mut results);
        }
        
        // Sort results by key for correct ordering
        results.sort_by(|a, b| a.0.cmp(&b.0));
        
        Self {
            results: results.into_iter(),
        }
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
        node: &Node<V>,
        start: &Bound<Vec<u8>>,
        end: &Bound<Vec<u8>>,
        results: &mut Vec<(Vec<u8>, V)>,
    ) where
        V: Clone,
    {
        match node {
            Node::Leaf { key, value } => {
                if Self::in_range(key, start, end) {
                    results.push((key.clone(), value.clone()));
                }
            }

            Node::Node4 { children, leaf_value, num_children, .. } => {
                // First, check leaf value at this node
                if let Some((key, leaf)) = leaf_value {
                    if let Node::Leaf { value, .. } = &**leaf {
                        if Self::in_range(key, start, end) {
                            results.push((key.clone(), value.clone()));
                        }
                    }
                }
                
                // Then visit all children
                for i in 0..(*num_children as usize) {
                    Self::collect_range(&children[i], start, end, results);
                }
            }

            Node::Node16 { children, leaf_value, num_children, .. } => {
                if let Some((key, leaf)) = leaf_value {
                    if let Node::Leaf { value, .. } = &**leaf {
                        if Self::in_range(key, start, end) {
                            results.push((key.clone(), value.clone()));
                        }
                    }
                }
                
                for i in 0..(*num_children as usize) {
                    Self::collect_range(&children[i], start, end, results);
                }
            }

            Node::Node48 { child_index, children, leaf_value, num_children, .. } => {
                if let Some((key, leaf)) = leaf_value {
                    if let Node::Leaf { value, .. } = &**leaf {
                        if Self::in_range(key, start, end) {
                            results.push((key.clone(), value.clone()));
                        }
                    }
                }
                
                // Visit children in byte order
                for byte in 0..=255u8 {
                    let idx = child_index[byte as usize];
                    if idx != 255 && (idx as usize) < children.len() {
                        Self::collect_range(&children[idx as usize], start, end, results);
                    }
                }
            }

            Node::Node256 { children, leaf_value, .. } => {
                if let Some((key, leaf)) = leaf_value {
                    if let Node::Leaf { value, .. } = &**leaf {
                        if Self::in_range(key, start, end) {
                            results.push((key.clone(), value.clone()));
                        }
                    }
                }
                
                for byte in 0..256 {
                    if let Some(ref child) = children[byte] {
                        Self::collect_range(child, start, end, results);
                    }
                }
            }
        }
    }
}

impl<V: Clone> Iterator for RangeIterator<V> {
    type Item = (Vec<u8>, V);

    fn next(&mut self) -> Option<Self::Item> {
        self.results.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_art_insert_get() {
        let mut tree: AdaptiveRadixTree<u64> = AdaptiveRadixTree::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        tree.insert(b"help", 3);
        
        assert_eq!(tree.get(b"hello"), Some(&1));
        assert_eq!(tree.get(b"world"), Some(&2));
        assert_eq!(tree.get(b"help"), Some(&3));
        assert_eq!(tree.get(b"hell"), None);
        assert_eq!(tree.get(b"helper"), None);
    }

    #[test]
    fn test_art_update() {
        let mut tree: AdaptiveRadixTree<u64> = AdaptiveRadixTree::new();
        
        assert!(tree.insert(b"key", 1).is_none());
        assert_eq!(tree.insert(b"key", 2), Some(1));
        assert_eq!(tree.get(b"key"), Some(&2));
    }

    #[test]
    fn test_art_remove() {
        let mut tree: AdaptiveRadixTree<u64> = AdaptiveRadixTree::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        
        assert_eq!(tree.remove(b"hello"), Some(1));
        assert_eq!(tree.get(b"hello"), None);
        assert_eq!(tree.get(b"world"), Some(&2));
    }

    #[test]
    fn test_art_prefix_sharing() {
        let mut tree: AdaptiveRadixTree<u64> = AdaptiveRadixTree::new();
        
        tree.insert(b"user:1001", 1);
        tree.insert(b"user:1002", 2);
        tree.insert(b"user:1003", 3);
        tree.insert(b"post:1001", 100);
        
        assert_eq!(tree.get(b"user:1001"), Some(&1));
        assert_eq!(tree.get(b"user:1002"), Some(&2));
        assert_eq!(tree.get(b"user:1003"), Some(&3));
        assert_eq!(tree.get(b"post:1001"), Some(&100));
    }

    #[test]
    fn test_art_prefix_scan() {
        let mut tree: AdaptiveRadixTree<u64> = AdaptiveRadixTree::new();
        
        tree.insert(b"user:1001", 1);
        tree.insert(b"user:1002", 2);
        tree.insert(b"user:1003", 3);
        tree.insert(b"post:1001", 100);
        
        let users: Vec<_> = tree.prefix_scan(b"user:").collect();
        assert_eq!(users.len(), 3);
    }

    #[test]
    fn test_art_empty_key() {
        let mut tree: AdaptiveRadixTree<u64> = AdaptiveRadixTree::new();
        
        tree.insert(b"", 42);
        assert_eq!(tree.get(b""), Some(&42));
        
        tree.insert(b"a", 1);
        assert_eq!(tree.get(b""), Some(&42));
        assert_eq!(tree.get(b"a"), Some(&1));
    }
}
