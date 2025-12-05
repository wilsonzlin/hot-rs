//! Lean ART: Minimal node count through aggressive path compression.
//!
//! Key insight: Most overhead comes from having too many nodes.
//! Solution: Never create leaf nodes - store values inline.

/// 4-byte node reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct NRef(u32);

impl NRef {
    pub const NULL: NRef = NRef(u32::MAX);
    
    #[inline(always)]
    pub fn is_null(self) -> bool { self.0 == u32::MAX }
    
    #[inline(always)]
    fn new(idx: usize) -> Self { NRef(idx as u32) }
    
    #[inline(always)]
    fn idx(self) -> usize { self.0 as usize }
}

/// Lean node - uses Box for larger arrays to minimize enum size.
/// Target: ~40 bytes per node.
#[derive(Clone)]
pub enum LeanNode<V: Clone> {
    /// 1-4 children with inline 8-byte prefix.
    N4 {
        prefix: [u8; 8],
        prefix_len: u8,
        count: u8,
        keys: [u8; 4],
        children: [NRef; 4],
        /// Value for key ending at this node (key stored in arena).
        value: Option<(u32, u16, V)>, // (offset, len, value)
    },
    
    /// 5-16 children.
    N16 {
        prefix_off: u32,
        prefix_len: u16,
        count: u8,
        keys: [u8; 16],
        children: Box<[NRef; 16]>,
        value: Option<(u32, u16, V)>,
    },
    
    /// 17-48 children.
    N48 {
        prefix_off: u32,
        prefix_len: u16,
        count: u8,
        idx: Box<[u8; 256]>,
        children: Box<[NRef; 48]>,
        value: Option<(u32, u16, V)>,
    },
    
    /// 49-256 children.
    N256 {
        prefix_off: u32,
        prefix_len: u16,
        count: u16,
        children: Box<[NRef; 256]>,
        value: Option<(u32, u16, V)>,
    },
}

impl<V: Clone> Default for LeanNode<V> {
    fn default() -> Self {
        LeanNode::N4 {
            prefix: [0; 8],
            prefix_len: 0,
            count: 0,
            keys: [0; 4],
            children: [NRef::NULL; 4],
            value: None,
        }
    }
}

impl<V: Clone> LeanNode<V> {
    fn new_n4(prefix: &[u8]) -> Self {
        let mut p = [0u8; 8];
        let len = prefix.len().min(8);
        p[..len].copy_from_slice(&prefix[..len]);
        LeanNode::N4 {
            prefix: p,
            prefix_len: len as u8,
            count: 0,
            keys: [0; 4],
            children: [NRef::NULL; 4],
            value: None,
        }
    }
    
    fn count(&self) -> usize {
        match self {
            LeanNode::N4 { count, .. } => *count as usize,
            LeanNode::N16 { count, .. } => *count as usize,
            LeanNode::N48 { count, .. } => *count as usize,
            LeanNode::N256 { count, .. } => *count as usize,
        }
    }
    
    fn is_full(&self) -> bool {
        match self {
            LeanNode::N4 { count, .. } => *count >= 4,
            LeanNode::N16 { count, .. } => *count >= 16,
            LeanNode::N48 { count, .. } => *count >= 48,
            LeanNode::N256 { .. } => false,
        }
    }
    
    fn find(&self, byte: u8) -> Option<NRef> {
        match self {
            LeanNode::N4 { keys, count, children, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        return Some(children[i]);
                    }
                }
                None
            }
            LeanNode::N16 { keys, count, children, .. } => {
                for i in 0..*count as usize {
                    if keys[i] == byte {
                        return Some(children[i]);
                    }
                }
                None
            }
            LeanNode::N48 { idx, children, .. } => {
                let i = idx[byte as usize];
                if i != 255 { Some(children[i as usize]) } else { None }
            }
            LeanNode::N256 { children, .. } => {
                let c = children[byte as usize];
                if !c.is_null() { Some(c) } else { None }
            }
        }
    }
    
    fn add(&mut self, byte: u8, child: NRef) {
        match self {
            LeanNode::N4 { keys, count, children, .. } => {
                debug_assert!((*count as usize) < 4);
                let i = *count as usize;
                keys[i] = byte;
                children[i] = child;
                *count += 1;
            }
            LeanNode::N16 { keys, count, children, .. } => {
                debug_assert!((*count as usize) < 16);
                let i = *count as usize;
                keys[i] = byte;
                children[i] = child;
                *count += 1;
            }
            LeanNode::N48 { idx, count, children, .. } => {
                debug_assert!((*count as usize) < 48);
                let slot = *count as usize;
                children[slot] = child;
                idx[byte as usize] = slot as u8;
                *count += 1;
            }
            LeanNode::N256 { children, count, .. } => {
                if children[byte as usize].is_null() {
                    *count += 1;
                }
                children[byte as usize] = child;
            }
        }
    }
    
    fn grow(self, keys_arena: &mut Vec<u8>) -> Self {
        match self {
            LeanNode::N4 { prefix, prefix_len, count, keys, children, value } => {
                // Store prefix in arena
                let prefix_off = keys_arena.len() as u32;
                keys_arena.extend_from_slice(&prefix[..prefix_len as usize]);
                
                let mut new_keys = [0u8; 16];
                let mut new_children = Box::new([NRef::NULL; 16]);
                for i in 0..count as usize {
                    new_keys[i] = keys[i];
                    new_children[i] = children[i];
                }
                LeanNode::N16 {
                    prefix_off,
                    prefix_len: prefix_len as u16,
                    count,
                    keys: new_keys,
                    children: new_children,
                    value,
                }
            }
            LeanNode::N16 { prefix_off, prefix_len, count, keys, children, value } => {
                let mut idx = Box::new([255u8; 256]);
                let mut new_children = Box::new([NRef::NULL; 48]);
                for i in 0..count as usize {
                    idx[keys[i] as usize] = i as u8;
                    new_children[i] = children[i];
                }
                LeanNode::N48 {
                    prefix_off,
                    prefix_len,
                    count,
                    idx,
                    children: new_children,
                    value,
                }
            }
            LeanNode::N48 { prefix_off, prefix_len, count, idx, children, value } => {
                let mut new_children = Box::new([NRef::NULL; 256]);
                for b in 0..256 {
                    let i = idx[b];
                    if i != 255 {
                        new_children[b] = children[i as usize];
                    }
                }
                LeanNode::N256 {
                    prefix_off,
                    prefix_len,
                    count: count as u16,
                    children: new_children,
                    value,
                }
            }
            other => other
        }
    }
}

/// Memory statistics.
#[derive(Debug, Clone, Default)]
pub struct LeanStats {
    pub key_bytes: usize,
    pub node_count: usize,
    pub node_bytes: usize,
}

/// Lean ART - minimal memory footprint.
pub struct LeanArt<V: Clone> {
    keys: Vec<u8>,
    nodes: Vec<LeanNode<V>>,
    root: NRef,
    len: usize,
}

impl<V: Clone> LeanArt<V> {
    pub fn new() -> Self {
        Self {
            keys: Vec::with_capacity(64 * 1024),
            nodes: Vec::with_capacity(1024),
            root: NRef::NULL,
            len: 0,
        }
    }
    
    fn store(&mut self, data: &[u8]) -> (u32, u16) {
        let off = self.keys.len() as u32;
        self.keys.extend_from_slice(data);
        (off, data.len() as u16)
    }
    
    fn load(&self, off: u32, len: u16) -> &[u8] {
        &self.keys[off as usize..(off as usize + len as usize)]
    }
    
    fn alloc(&mut self, node: LeanNode<V>) -> NRef {
        let idx = self.nodes.len();
        self.nodes.push(node);
        NRef::new(idx)
    }
    
    fn node(&self, r: NRef) -> &LeanNode<V> {
        &self.nodes[r.idx()]
    }
    
    fn node_mut(&mut self, r: NRef) -> &mut LeanNode<V> {
        &mut self.nodes[r.idx()]
    }
    
    fn get_prefix(&self, node: &LeanNode<V>) -> Vec<u8> {
        match node {
            LeanNode::N4 { prefix, prefix_len, .. } => {
                prefix[..*prefix_len as usize].to_vec()
            }
            LeanNode::N16 { prefix_off, prefix_len, .. } |
            LeanNode::N48 { prefix_off, prefix_len, .. } |
            LeanNode::N256 { prefix_off, prefix_len, .. } => {
                self.load(*prefix_off, *prefix_len).to_vec()
            }
        }
    }
    
    fn set_prefix(&mut self, r: NRef, new_prefix: &[u8]) {
        // Check node type first
        let is_n4 = matches!(self.node(r), LeanNode::N4 { .. });
        
        if is_n4 {
            if let LeanNode::N4 { prefix, prefix_len, .. } = self.node_mut(r) {
                let len = new_prefix.len().min(8);
                prefix[..len].copy_from_slice(&new_prefix[..len]);
                *prefix_len = len as u8;
            }
        } else {
            let (off, len) = self.store(new_prefix);
            match self.node_mut(r) {
                LeanNode::N16 { prefix_off, prefix_len, .. } |
                LeanNode::N48 { prefix_off, prefix_len, .. } |
                LeanNode::N256 { prefix_off, prefix_len, .. } => {
                    *prefix_off = off;
                    *prefix_len = len;
                }
                _ => {}
            }
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        let (key_off, key_len) = self.store(key);
        
        if self.root.is_null() {
            // First key - just create a node with this value
            let mut node = LeanNode::new_n4(&[]);
            node = match node {
                LeanNode::N4 { prefix, prefix_len, count, keys, children, .. } => {
                    LeanNode::N4 {
                        prefix, prefix_len, count, keys, children,
                        value: Some((key_off, key_len, value)),
                    }
                }
                _ => unreachable!()
            };
            self.root = self.alloc(node);
            self.len = 1;
            return None;
        }
        
        let result = self.insert_impl(key, key_off, key_len, value);
        if result.is_none() {
            self.len += 1;
        }
        result
    }
    
    fn insert_impl(&mut self, key: &[u8], key_off: u32, key_len: u16, value: V) -> Option<V> {
        let mut path: Vec<(NRef, u8)> = Vec::new();
        let mut cur = self.root;
        let mut depth = 0;
        
        loop {
            let prefix = self.get_prefix(self.node(cur));
            let prefix_len = prefix.len();
            
            // Check prefix match
            if prefix_len > 0 {
                let remaining = &key[depth..];
                if remaining.len() < prefix_len || &remaining[..prefix_len] != prefix.as_slice() {
                    // Mismatch - split
                    let mismatch = remaining.iter()
                        .zip(prefix.iter())
                        .take_while(|(a, b)| a == b)
                        .count();
                    
                    // Create split node
                    let mut split = LeanNode::new_n4(&prefix[..mismatch]);
                    
                    // Current node gets remaining prefix
                    let remaining_prefix = prefix[mismatch + 1..].to_vec();
                    self.set_prefix(cur, &remaining_prefix);
                    
                    // Add current as child
                    let existing_byte = prefix[mismatch];
                    split.add(existing_byte, cur);
                    
                    // Handle new key
                    if depth + mismatch >= key.len() {
                        // Key ends here
                        split = match split {
                            LeanNode::N4 { prefix, prefix_len, count, keys, children, .. } => {
                                LeanNode::N4 {
                                    prefix, prefix_len, count, keys, children,
                                    value: Some((key_off, key_len, value)),
                                }
                            }
                            _ => unreachable!()
                        };
                    } else {
                        // New key continues
                        let new_byte = key[depth + mismatch];
                        let mut new_node = LeanNode::new_n4(&[]);
                        new_node = match new_node {
                            LeanNode::N4 { prefix, prefix_len, count, keys, children, .. } => {
                                LeanNode::N4 {
                                    prefix, prefix_len, count, keys, children,
                                    value: Some((key_off, key_len, value)),
                                }
                            }
                            _ => unreachable!()
                        };
                        let new_ref = self.alloc(new_node);
                        split.add(new_byte, new_ref);
                    }
                    
                    let split_ref = self.alloc(split);
                    
                    if path.is_empty() {
                        self.root = split_ref;
                    } else {
                        let (parent, byte) = path.last().unwrap();
                        match self.node_mut(*parent) {
                            LeanNode::N4 { keys, children, count, .. } => {
                                for i in 0..*count as usize {
                                    if keys[i] == *byte {
                                        children[i] = split_ref;
                                        break;
                                    }
                                }
                            }
                            LeanNode::N16 { keys, children, count, .. } => {
                                for i in 0..*count as usize {
                                    if keys[i] == *byte {
                                        children[i] = split_ref;
                                        break;
                                    }
                                }
                            }
                            LeanNode::N48 { idx, children, .. } => {
                                let i = idx[*byte as usize];
                                children[i as usize] = split_ref;
                            }
                            LeanNode::N256 { children, .. } => {
                                children[*byte as usize] = split_ref;
                            }
                        }
                    }
                    
                    return None;
                }
            }
            
            depth += prefix_len;
            
            // Key ends at this node?
            if depth >= key.len() {
                let old = match self.node_mut(cur) {
                    LeanNode::N4 { value: v, .. } |
                    LeanNode::N16 { value: v, .. } |
                    LeanNode::N48 { value: v, .. } |
                    LeanNode::N256 { value: v, .. } => {
                        let old = v.take().map(|(_, _, old_v)| old_v);
                        *v = Some((key_off, key_len, value));
                        old
                    }
                };
                return old;
            }
            
            // Find child
            let next_byte = key[depth];
            let child = self.node(cur).find(next_byte);
            
            if let Some(c) = child {
                path.push((cur, next_byte));
                cur = c;
                depth += 1;
            } else {
                // Add new child
                if self.node(cur).is_full() {
                    let old = std::mem::take(self.node_mut(cur));
                    *self.node_mut(cur) = old.grow(&mut self.keys);
                }
                
                let mut new_node = LeanNode::new_n4(&[]);
                new_node = match new_node {
                    LeanNode::N4 { prefix, prefix_len, count, keys, children, .. } => {
                        LeanNode::N4 {
                            prefix, prefix_len, count, keys, children,
                            value: Some((key_off, key_len, value)),
                        }
                    }
                    _ => unreachable!()
                };
                let new_ref = self.alloc(new_node);
                self.node_mut(cur).add(next_byte, new_ref);
                
                return None;
            }
        }
    }
    
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.root.is_null() {
            return None;
        }
        
        let mut cur = self.root;
        let mut depth = 0;
        
        loop {
            let node = self.node(cur);
            let prefix = self.get_prefix(node);
            
            // Check prefix
            if prefix.len() > 0 {
                if key.len() < depth + prefix.len() || &key[depth..depth + prefix.len()] != prefix.as_slice() {
                    return None;
                }
            }
            depth += prefix.len();
            
            // Key ends here?
            if depth >= key.len() {
                return match node {
                    LeanNode::N4 { value, .. } |
                    LeanNode::N16 { value, .. } |
                    LeanNode::N48 { value, .. } |
                    LeanNode::N256 { value, .. } => {
                        value.as_ref().and_then(|(off, len, v)| {
                            if self.load(*off, *len) == key {
                                Some(v)
                            } else {
                                None
                            }
                        })
                    }
                };
            }
            
            // Find child
            let next_byte = key[depth];
            if let Some(c) = node.find(next_byte) {
                cur = c;
                depth += 1;
            } else {
                return None;
            }
        }
    }
    
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    pub fn stats(&self) -> LeanStats {
        LeanStats {
            key_bytes: self.keys.capacity(),
            node_count: self.nodes.len(),
            node_bytes: self.nodes.capacity() * std::mem::size_of::<LeanNode<V>>(),
        }
    }
}

impl<V: Clone> Default for LeanArt<V> {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut t: LeanArt<u64> = LeanArt::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        t.insert(b"he", 3);
        
        assert_eq!(t.get(b"hello"), Some(&1));
        assert_eq!(t.get(b"world"), Some(&2));
        assert_eq!(t.get(b"he"), Some(&3));
        assert_eq!(t.get(b"hel"), None);
    }
    
    #[test]
    fn test_many() {
        let mut t: LeanArt<u64> = LeanArt::new();
        for i in 0..1000u64 {
            t.insert(format!("key{:05}", i).as_bytes(), i);
        }
        for i in 0..1000u64 {
            assert_eq!(t.get(format!("key{:05}", i).as_bytes()), Some(&i));
        }
    }
    
    #[test]
    fn test_size() {
        println!("LeanNode<u64>: {} bytes", std::mem::size_of::<LeanNode<u64>>());
    }
}
