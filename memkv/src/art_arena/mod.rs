//! Arena-based ART with minimal allocation overhead.
//!
//! This implementation stores all nodes in a single arena, using 32-bit indices
//! instead of 64-bit pointers. This eliminates per-node allocation overhead.

/// A 32-bit reference to a node in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct NodeRef(u32);

impl NodeRef {
    pub const NULL: NodeRef = NodeRef(u32::MAX);
    
    #[inline]
    pub fn is_null(self) -> bool {
        self.0 == u32::MAX
    }
    
    #[inline]
    fn new(idx: usize) -> Self {
        debug_assert!(idx < u32::MAX as usize);
        NodeRef(idx as u32)
    }
    
    #[inline]
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// A compact 6-byte reference to data in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(C, packed)]
pub struct DataRef {
    offset: u32,
    len: u16,
}

impl DataRef {
    #[inline]
    pub const fn empty() -> Self {
        Self { offset: 0, len: 0 }
    }
    
    #[inline]
    pub fn new(offset: usize, len: usize) -> Self {
        debug_assert!(offset <= u32::MAX as usize);
        debug_assert!(len <= u16::MAX as usize);
        Self { 
            offset: offset as u32, 
            len: len as u16 
        }
    }
    
    #[inline]
    pub fn offset(self) -> usize {
        self.offset as usize
    }
    
    #[inline]
    pub fn len(self) -> usize {
        self.len as usize
    }
    
    #[inline]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// A node in the arena-based ART.
/// Optimized for small enum size by boxing large arrays.
#[derive(Clone)]
pub enum ArenaNode<V: Clone> {
    /// A leaf node stores a key reference and value.
    Leaf {
        key_ref: DataRef,
        value: V,
    },
    
    /// Node4: 1-4 children, stored inline (small).
    Node4 {
        prefix_ref: DataRef,
        num_children: u8,
        keys: [u8; 4],
        children: [NodeRef; 4],
        /// Value for keys that end at this node.
        leaf: Option<(DataRef, V)>,
    },
    
    /// Node16: 5-16 children, children boxed to reduce enum size.
    Node16 {
        prefix_ref: DataRef,
        num_children: u8,
        keys: [u8; 16],
        children: Box<[NodeRef; 16]>,
        leaf: Option<(DataRef, V)>,
    },
    
    /// Node48: 17-48 children, both arrays boxed.
    Node48 {
        prefix_ref: DataRef,
        num_children: u8,
        /// Maps byte value to child index (255 = empty).
        child_index: Box<[u8; 256]>,
        children: Box<[NodeRef; 48]>,
        leaf: Option<(DataRef, V)>,
    },
    
    /// Node256: 49-256 children, direct indexing.
    Node256 {
        prefix_ref: DataRef,
        num_children: u16,
        children: Box<[NodeRef; 256]>,
        leaf: Option<(DataRef, V)>,
    },
}

impl<V: Clone> Default for ArenaNode<V> {
    fn default() -> Self {
        ArenaNode::Node4 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            keys: [0; 4],
            children: [NodeRef::NULL; 4],
            leaf: None,
        }
    }
}

impl<V: Clone> ArenaNode<V> {
    #[inline]
    pub fn new_leaf(key_ref: DataRef, value: V) -> Self {
        ArenaNode::Leaf { key_ref, value }
    }
    
    #[inline]
    pub fn new_node4() -> Self {
        ArenaNode::Node4 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            keys: [0; 4],
            children: [NodeRef::NULL; 4],
            leaf: None,
        }
    }
    
    pub fn new_node16() -> Self {
        ArenaNode::Node16 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            keys: [0; 16],
            children: Box::new([NodeRef::NULL; 16]),
            leaf: None,
        }
    }
    
    pub fn new_node48() -> Self {
        ArenaNode::Node48 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            child_index: Box::new([255; 256]),
            children: Box::new([NodeRef::NULL; 48]),
            leaf: None,
        }
    }
    
    pub fn new_node256() -> Self {
        ArenaNode::Node256 {
            prefix_ref: DataRef::empty(),
            num_children: 0,
            children: Box::new([NodeRef::NULL; 256]),
            leaf: None,
        }
    }
    
    #[inline]
    pub fn is_leaf(&self) -> bool {
        matches!(self, ArenaNode::Leaf { .. })
    }
    
    #[inline]
    pub fn prefix_ref(&self) -> DataRef {
        match self {
            ArenaNode::Leaf { .. } => DataRef::empty(),
            ArenaNode::Node4 { prefix_ref, .. }
            | ArenaNode::Node16 { prefix_ref, .. }
            | ArenaNode::Node48 { prefix_ref, .. }
            | ArenaNode::Node256 { prefix_ref, .. } => *prefix_ref,
        }
    }
    
    #[inline]
    pub fn set_prefix_ref(&mut self, new_ref: DataRef) {
        match self {
            ArenaNode::Leaf { .. } => {}
            ArenaNode::Node4 { prefix_ref, .. }
            | ArenaNode::Node16 { prefix_ref, .. }
            | ArenaNode::Node48 { prefix_ref, .. }
            | ArenaNode::Node256 { prefix_ref, .. } => {
                *prefix_ref = new_ref;
            }
        }
    }
    
    #[inline]
    pub fn num_children(&self) -> usize {
        match self {
            ArenaNode::Leaf { .. } => 0,
            ArenaNode::Node4 { num_children, .. } => *num_children as usize,
            ArenaNode::Node16 { num_children, .. } => *num_children as usize,
            ArenaNode::Node48 { num_children, .. } => *num_children as usize,
            ArenaNode::Node256 { num_children, .. } => *num_children as usize,
        }
    }
    
    #[inline]
    pub fn find_child(&self, key: u8) -> Option<NodeRef> {
        match self {
            ArenaNode::Leaf { .. } => None,
            ArenaNode::Node4 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(children[i]);
                    }
                }
                None
            }
            ArenaNode::Node16 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        return Some(children[i]);
                    }
                }
                None
            }
            ArenaNode::Node48 { child_index, children, .. } => {
                let idx = child_index[key as usize];
                if idx != 255 {
                    Some(children[idx as usize])
                } else {
                    None
                }
            }
            ArenaNode::Node256 { children, .. } => {
                let child = children[key as usize];
                if !child.is_null() {
                    Some(child)
                } else {
                    None
                }
            }
        }
    }
    
    pub fn set_child(&mut self, key: u8, child: NodeRef) {
        match self {
            ArenaNode::Leaf { .. } => panic!("Cannot set child on leaf"),
            ArenaNode::Node4 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = child;
                        return;
                    }
                }
                debug_assert!((*num_children as usize) < 4, "Node4 is full");
                let idx = *num_children as usize;
                keys[idx] = key;
                children[idx] = child;
                *num_children += 1;
            }
            ArenaNode::Node16 { keys, num_children, children, .. } => {
                for i in 0..*num_children as usize {
                    if keys[i] == key {
                        children[i] = child;
                        return;
                    }
                }
                debug_assert!((*num_children as usize) < 16, "Node16 is full");
                let idx = *num_children as usize;
                keys[idx] = key;
                children[idx] = child;
                *num_children += 1;
            }
            ArenaNode::Node48 { child_index, num_children, children, .. } => {
                let existing = child_index[key as usize];
                if existing != 255 {
                    children[existing as usize] = child;
                } else {
                    debug_assert!((*num_children as usize) < 48, "Node48 is full");
                    let slot = *num_children as usize;
                    children[slot] = child;
                    child_index[key as usize] = slot as u8;
                    *num_children += 1;
                }
            }
            ArenaNode::Node256 { children, num_children, .. } => {
                if children[key as usize].is_null() {
                    *num_children += 1;
                }
                children[key as usize] = child;
            }
        }
    }
    
    #[inline]
    pub fn should_grow(&self) -> bool {
        match self {
            ArenaNode::Leaf { .. } => false,
            ArenaNode::Node4 { num_children, .. } => *num_children >= 4,
            ArenaNode::Node16 { num_children, .. } => *num_children >= 16,
            ArenaNode::Node48 { num_children, .. } => *num_children >= 48,
            ArenaNode::Node256 { .. } => false,
        }
    }
    
    pub fn grow(self) -> Self {
        match self {
            ArenaNode::Node4 { prefix_ref, num_children, keys, children, leaf } => {
                let mut new_keys = [0u8; 16];
                let mut new_children = Box::new([NodeRef::NULL; 16]);
                for i in 0..num_children as usize {
                    new_keys[i] = keys[i];
                    new_children[i] = children[i];
                }
                ArenaNode::Node16 {
                    prefix_ref,
                    num_children,
                    keys: new_keys,
                    children: new_children,
                    leaf,
                }
            }
            ArenaNode::Node16 { prefix_ref, num_children, keys, children, leaf } => {
                let mut child_index = Box::new([255u8; 256]);
                let mut new_children = Box::new([NodeRef::NULL; 48]);
                for i in 0..num_children as usize {
                    child_index[keys[i] as usize] = i as u8;
                    new_children[i] = children[i];
                }
                ArenaNode::Node48 {
                    prefix_ref,
                    num_children,
                    child_index,
                    children: new_children,
                    leaf,
                }
            }
            ArenaNode::Node48 { prefix_ref, num_children, child_index, children, leaf } => {
                let mut new_children = Box::new([NodeRef::NULL; 256]);
                for byte in 0..256 {
                    let idx = child_index[byte];
                    if idx != 255 {
                        new_children[byte] = children[idx as usize];
                    }
                }
                ArenaNode::Node256 {
                    prefix_ref,
                    num_children: num_children as u16,
                    children: new_children,
                    leaf,
                }
            }
            other => other,
        }
    }
    
    #[inline]
    pub fn leaf_value(&self) -> Option<&(DataRef, V)> {
        match self {
            ArenaNode::Leaf { .. } => None,
            ArenaNode::Node4 { leaf, .. }
            | ArenaNode::Node16 { leaf, .. }
            | ArenaNode::Node48 { leaf, .. }
            | ArenaNode::Node256 { leaf, .. } => leaf.as_ref(),
        }
    }
    
    #[inline]
    pub fn set_leaf(&mut self, new_leaf: Option<(DataRef, V)>) {
        match self {
            ArenaNode::Leaf { .. } => {}
            ArenaNode::Node4 { leaf, .. }
            | ArenaNode::Node16 { leaf, .. }
            | ArenaNode::Node48 { leaf, .. }
            | ArenaNode::Node256 { leaf, .. } => {
                *leaf = new_leaf;
            }
        }
    }
    
    pub fn take_leaf(&mut self) -> Option<(DataRef, V)> {
        match self {
            ArenaNode::Leaf { .. } => None,
            ArenaNode::Node4 { leaf, .. }
            | ArenaNode::Node16 { leaf, .. }
            | ArenaNode::Node48 { leaf, .. }
            | ArenaNode::Node256 { leaf, .. } => leaf.take(),
        }
    }
    
    /// Get all children as (key, child_ref) pairs.
    pub fn children(&self) -> Vec<(u8, NodeRef)> {
        match self {
            ArenaNode::Leaf { .. } => vec![],
            ArenaNode::Node4 { keys, num_children, children, .. } => {
                (0..*num_children as usize)
                    .map(|i| (keys[i], children[i]))
                    .collect()
            }
            ArenaNode::Node16 { keys, num_children, children, .. } => {
                (0..*num_children as usize)
                    .map(|i| (keys[i], children[i]))
                    .collect()
            }
            ArenaNode::Node48 { child_index, children, .. } => {
                (0..256u16)
                    .filter_map(|b| {
                        let idx = child_index[b as usize];
                        if idx != 255 {
                            Some((b as u8, children[idx as usize]))
                        } else {
                            None
                        }
                    })
                    .collect()
            }
            ArenaNode::Node256 { children, .. } => {
                (0..256u16)
                    .filter_map(|b| {
                        let child = children[b as usize];
                        if !child.is_null() {
                            Some((b as u8, child))
                        } else {
                            None
                        }
                    })
                    .collect()
            }
        }
    }
}

/// Memory statistics.
#[derive(Debug, Clone, Default)]
pub struct ArenaArtStats {
    pub data_arena_bytes: usize,
    pub node_arena_capacity: usize,
    pub node_count: usize,
    pub leaf_count: usize,
    pub node4_count: usize,
    pub node16_count: usize,
    pub node48_count: usize,
    pub node256_count: usize,
}

/// Arena-based Adaptive Radix Tree.
pub struct ArenaArt<V: Clone> {
    /// All nodes stored contiguously.
    nodes: Vec<ArenaNode<V>>,
    /// Arena for keys and prefixes.
    data: Vec<u8>,
    /// Root node reference.
    root: NodeRef,
    /// Number of keys stored.
    size: usize,
}

impl<V: Clone> ArenaArt<V> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(1024),
            data: Vec::with_capacity(64 * 1024),
            root: NodeRef::NULL,
            size: 0,
        }
    }
    
    /// Allocate a new node and return its reference.
    #[inline]
    fn alloc(&mut self, node: ArenaNode<V>) -> NodeRef {
        let idx = self.nodes.len();
        self.nodes.push(node);
        NodeRef::new(idx)
    }
    
    /// Store bytes in data arena.
    #[inline]
    fn store_data(&mut self, bytes: &[u8]) -> DataRef {
        if bytes.is_empty() {
            return DataRef::empty();
        }
        let offset = self.data.len();
        self.data.extend_from_slice(bytes);
        DataRef::new(offset, bytes.len())
    }
    
    /// Get bytes from data arena.
    #[inline]
    fn get_data(&self, data_ref: DataRef) -> &[u8] {
        if data_ref.is_empty() {
            return &[];
        }
        &self.data[data_ref.offset()..data_ref.offset() + data_ref.len()]
    }
    
    /// Get node by reference.
    #[inline]
    fn node(&self, idx: NodeRef) -> &ArenaNode<V> {
        &self.nodes[idx.index()]
    }
    
    /// Get mutable node by reference.
    #[inline]
    fn node_mut(&mut self, idx: NodeRef) -> &mut ArenaNode<V> {
        &mut self.nodes[idx.index()]
    }
    
    /// Insert a key-value pair.
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        let key_ref = self.store_data(key);
        
        if self.root.is_null() {
            let leaf = ArenaNode::new_leaf(key_ref, value);
            self.root = self.alloc(leaf);
            self.size = 1;
            return None;
        }
        
        let result = self.insert_impl(key, key_ref, value);
        if result.is_none() {
            self.size += 1;
        }
        result
    }
    
    fn insert_impl(&mut self, key: &[u8], key_ref: DataRef, value: V) -> Option<V> {
        // Track the path through the tree
        let mut path: Vec<(NodeRef, u8)> = Vec::with_capacity(key.len() / 4);
        let mut current = self.root;
        let mut depth = 0;
        
        loop {
            // First, gather all information we need without holding borrows
            let (is_leaf, leaf_key_ref_opt, prefix_ref) = {
                let node = self.node(current);
                match node {
                    ArenaNode::Leaf { key_ref: lkr, .. } => (true, Some(*lkr), DataRef::empty()),
                    _ => (false, None, node.prefix_ref()),
                }
            };
            
            // Handle leaf node
            if is_leaf {
                let leaf_key_ref = leaf_key_ref_opt.unwrap();
                
                // Copy existing key to avoid borrow issues
                let leaf_key: Vec<u8> = self.get_data(leaf_key_ref).to_vec();
                
                // Same key - replace value
                if leaf_key.as_slice() == key {
                    if let ArenaNode::Leaf { value: v, .. } = self.node_mut(current) {
                        return Some(std::mem::replace(v, value));
                    }
                }
                
                // Different key - need to split
                let common_len = leaf_key[depth..]
                    .iter()
                    .zip(key[depth..].iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                
                let split_depth = depth + common_len;
                let existing_byte = leaf_key.get(split_depth).copied();
                let new_byte = key.get(split_depth).copied();
                
                // Create new inner node
                let mut new_inner = ArenaNode::new_node4();
                if common_len > 0 {
                    let prefix_ref = self.store_data(&key[depth..split_depth]);
                    new_inner.set_prefix_ref(prefix_ref);
                }
                
                // Clone the existing leaf's data before mutating
                let leaf_data = if let ArenaNode::Leaf { key_ref: lkr, value: lv } = self.node(current).clone() {
                    Some((lkr, lv))
                } else {
                    None
                };
                
                match (existing_byte, new_byte) {
                    (Some(eb), Some(nb)) => {
                        // Both have more bytes - both become children
                        let new_leaf = ArenaNode::new_leaf(key_ref, value);
                        let new_leaf_ref = self.alloc(new_leaf);
                        
                        new_inner.set_child(eb, current);
                        new_inner.set_child(nb, new_leaf_ref);
                    }
                    (Some(eb), None) => {
                        // New key ends here - store as leaf value
                        new_inner.set_child(eb, current);
                        new_inner.set_leaf(Some((key_ref, value)));
                    }
                    (None, Some(nb)) => {
                        // Existing key ends here - store as leaf value
                        if let Some((lkr, lv)) = leaf_data {
                            new_inner.set_leaf(Some((lkr, lv)));
                            
                            let new_leaf = ArenaNode::new_leaf(key_ref, value);
                            let new_leaf_ref = self.alloc(new_leaf);
                            new_inner.set_child(nb, new_leaf_ref);
                            
                            // Replace old leaf node with empty (it's now inline in parent)
                            *self.node_mut(current) = ArenaNode::new_node4();
                        }
                    }
                    (None, None) => {
                        // Keys are equal - should have been caught above
                        unreachable!()
                    }
                }
                
                let new_inner_ref = self.alloc(new_inner);
                
                // Update parent or root
                if path.is_empty() {
                    self.root = new_inner_ref;
                } else {
                    let (parent_ref, byte) = path.last().unwrap();
                    self.node_mut(*parent_ref).set_child(*byte, new_inner_ref);
                }
                
                return None;
            }
            
            // Handle internal node - copy prefix to avoid borrow issues
            let prefix: Vec<u8> = self.get_data(prefix_ref).to_vec();
            let prefix_len = prefix.len();
            
            // Check prefix match
            if prefix_len > 0 {
                let key_remaining = &key[depth..];
                if key_remaining.len() < prefix_len || &key_remaining[..prefix_len] != prefix.as_slice() {
                    // Prefix mismatch - need to split
                    let mismatch_idx = key_remaining
                        .iter()
                        .zip(prefix.iter())
                        .take_while(|(a, b)| a == b)
                        .count();
                    
                    // Create new inner node at mismatch point
                    let mut split_node = ArenaNode::new_node4();
                    if mismatch_idx > 0 {
                        let split_prefix = self.store_data(&prefix[..mismatch_idx]);
                        split_node.set_prefix_ref(split_prefix);
                    }
                    
                    // Existing node becomes child with remaining prefix
                    let existing_byte = prefix[mismatch_idx];
                    let remaining_prefix = if mismatch_idx + 1 < prefix_len {
                        self.store_data(&prefix[mismatch_idx + 1..])
                    } else {
                        DataRef::empty()
                    };
                    self.node_mut(current).set_prefix_ref(remaining_prefix);
                    split_node.set_child(existing_byte, current);
                    
                    // Handle new key
                    if depth + mismatch_idx >= key.len() {
                        // New key ends at split point
                        split_node.set_leaf(Some((key_ref, value)));
                    } else {
                        // New key continues
                        let new_byte = key[depth + mismatch_idx];
                        let new_leaf = ArenaNode::new_leaf(key_ref, value);
                        let new_leaf_ref = self.alloc(new_leaf);
                        split_node.set_child(new_byte, new_leaf_ref);
                    }
                    
                    let split_ref = self.alloc(split_node);
                    
                    if path.is_empty() {
                        self.root = split_ref;
                    } else {
                        let (parent_ref, byte) = path.last().unwrap();
                        self.node_mut(*parent_ref).set_child(*byte, split_ref);
                    }
                    
                    return None;
                }
            }
            
            depth += prefix_len;
            
            // Check if key ends at this node
            if depth >= key.len() {
                // Key ends here - store as leaf value
                let old_value_opt = self.node(current).leaf_value().map(|(_, v)| v.clone());
                self.node_mut(current).set_leaf(Some((key_ref, value)));
                return old_value_opt;
            }
            
            // Continue to child
            let next_byte = key[depth];
            let child_opt = self.node(current).find_child(next_byte);
            
            if let Some(child) = child_opt {
                path.push((current, next_byte));
                current = child;
                depth += 1;
            } else {
                // No child - add new leaf
                let new_leaf = ArenaNode::new_leaf(key_ref, value);
                let new_leaf_ref = self.alloc(new_leaf);
                
                // May need to grow node
                if self.node(current).should_grow() {
                    let grown = self.node_mut(current).clone().grow();
                    *self.node_mut(current) = grown;
                }
                
                self.node_mut(current).set_child(next_byte, new_leaf_ref);
                return None;
            }
        }
    }
    
    /// Get a reference to the value for a key.
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.root.is_null() {
            return None;
        }
        
        let mut current = self.root;
        let mut depth = 0;
        
        loop {
            let node = self.node(current);
            
            match node {
                ArenaNode::Leaf { key_ref, value } => {
                    let stored_key = self.get_data(*key_ref);
                    if stored_key == key {
                        return Some(value);
                    }
                    return None;
                }
                _ => {
                    let prefix = self.get_data(node.prefix_ref());
                    let prefix_len = prefix.len();
                    
                    if prefix_len > 0 {
                        if key.len() < depth + prefix_len 
                            || &key[depth..depth + prefix_len] != prefix 
                        {
                            return None;
                        }
                    }
                    depth += prefix_len;
                    
                    if depth >= key.len() {
                        if let Some((kr, value)) = node.leaf_value() {
                            if self.get_data(*kr) == key {
                                return Some(value);
                            }
                        }
                        return None;
                    }
                    
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
    pub fn memory_stats(&self) -> ArenaArtStats {
        let mut stats = ArenaArtStats {
            data_arena_bytes: self.data.capacity(),
            node_arena_capacity: self.nodes.capacity() * std::mem::size_of::<ArenaNode<V>>(),
            node_count: self.nodes.len(),
            ..Default::default()
        };
        
        for node in &self.nodes {
            match node {
                ArenaNode::Leaf { .. } => stats.leaf_count += 1,
                ArenaNode::Node4 { .. } => stats.node4_count += 1,
                ArenaNode::Node16 { .. } => stats.node16_count += 1,
                ArenaNode::Node48 { .. } => stats.node48_count += 1,
                ArenaNode::Node256 { .. } => stats.node256_count += 1,
            }
        }
        
        stats
    }
}

impl<V: Clone> Default for ArenaArt<V> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic_operations() {
        let mut tree: ArenaArt<u64> = ArenaArt::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        tree.insert(b"hell", 3);
        tree.insert(b"help", 4);
        
        assert_eq!(tree.get(b"hello"), Some(&1));
        assert_eq!(tree.get(b"world"), Some(&2));
        assert_eq!(tree.get(b"hell"), Some(&3));
        assert_eq!(tree.get(b"help"), Some(&4));
        assert_eq!(tree.get(b"hel"), None);
        assert_eq!(tree.get(b"hellox"), None);
    }
    
    #[test]
    fn test_replace() {
        let mut tree: ArenaArt<u64> = ArenaArt::new();
        
        assert_eq!(tree.insert(b"key", 1), None);
        assert_eq!(tree.insert(b"key", 2), Some(1));
        assert_eq!(tree.get(b"key"), Some(&2));
    }
    
    #[test]
    fn test_node_sizes() {
        println!("NodeRef: {} bytes", std::mem::size_of::<NodeRef>());
        println!("DataRef: {} bytes", std::mem::size_of::<DataRef>());
        println!("ArenaNode<u64>: {} bytes", std::mem::size_of::<ArenaNode<u64>>());
        println!("ArenaNode<u32>: {} bytes", std::mem::size_of::<ArenaNode<u32>>());
    }
    
    #[test]
    fn test_prefix_split() {
        let mut tree: ArenaArt<u64> = ArenaArt::new();
        
        tree.insert(b"abcdef", 1);
        tree.insert(b"abcxyz", 2);
        
        assert_eq!(tree.get(b"abcdef"), Some(&1));
        assert_eq!(tree.get(b"abcxyz"), Some(&2));
        assert_eq!(tree.get(b"abc"), None);
    }
    
    #[test]
    fn test_many_keys() {
        let mut tree: ArenaArt<u64> = ArenaArt::new();
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            assert_eq!(tree.get(key.as_bytes()), Some(&i));
        }
        
        assert_eq!(tree.len(), 1000);
    }
}
