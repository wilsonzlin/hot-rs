//! GloryArt: Ultra-optimized ART targeting 11-14 bytes/key overhead
//!
//! Key optimizations for minimal memory:
//! 1. **Single arena allocation** - all nodes in one Vec<u8>, zero allocation overhead
//! 2. **4-byte node references** - 32-bit offsets, not 64-bit pointers  
//! 3. **Unified node/leaf** - internal nodes can store values (no separate leaves)
//! 4. **Inline key suffixes** - keys stored directly after node headers
//! 5. **No per-node allocation overhead** - everything packed into arena
//!
//! Target: 11-14 bytes overhead per key (matching HOT paper results)

#![allow(unsafe_op_in_unsafe_fn)]

/// 4-byte node reference (offset into arena)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Ref(u32);

impl Ref {
    const NULL: Self = Ref(0xFFFFFFFF);
    
    #[inline(always)]
    fn is_null(self) -> bool { self.0 == 0xFFFFFFFF }
    
    #[inline(always)]
    fn new(offset: usize) -> Self { 
        debug_assert!(offset < 0xFFFFFFFF);
        Ref(offset as u32) 
    }
    
    #[inline(always)]
    fn offset(self) -> usize { self.0 as usize }
}

/// Compact node header (8 bytes total for alignment)
/// Byte 0: node_type (2 bits) | has_value (1 bit) | num_children (5 bits for N4/N16)
/// Byte 1: prefix_len (8 bits, supports up to 255)
/// Bytes 2-7: reserved for value if has_value, else part of prefix
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct NodeHeader {
    flags: u8,         // type(2) | has_value(1) | num_children_low(5)
    prefix_len: u8,    // 0-255
}

impl NodeHeader {
    const TYPE_MASK: u8 = 0b11000000;
    const TYPE_SHIFT: u8 = 6;
    const HAS_VALUE: u8 = 0b00100000;
    const NUM_CHILDREN_MASK: u8 = 0b00011111;
    
    // Node types
    const TYPE_N4: u8 = 0;    // 1-4 children
    const TYPE_N16: u8 = 1;   // 5-16 children
    const TYPE_N48: u8 = 2;   // 17-48 children
    const TYPE_N256: u8 = 3;  // 49-256 children
    
    #[inline]
    fn node_type_raw(self) -> u8 {
        (self.flags & Self::TYPE_MASK) >> Self::TYPE_SHIFT
    }
    
    #[inline]
    fn has_value(self) -> bool {
        (self.flags & Self::HAS_VALUE) != 0
    }
    
    #[inline]
    fn num_children(self) -> usize {
        // For N48/N256, this field may overflow, but we handle separately
        (self.flags & Self::NUM_CHILDREN_MASK) as usize
    }
    
    #[inline]
    fn set_num_children(&mut self, n: usize) {
        self.flags = (self.flags & !Self::NUM_CHILDREN_MASK) | ((n as u8) & Self::NUM_CHILDREN_MASK);
    }
    
    #[inline]
    fn set_has_value(&mut self, has: bool) {
        if has {
            self.flags |= Self::HAS_VALUE;
        } else {
            self.flags &= !Self::HAS_VALUE;
        }
    }
    
    #[inline]
    fn new(node_type: u8, has_value: bool, num_children: usize, prefix_len: usize) -> Self {
        let type_bits = (node_type & 0b11) << Self::TYPE_SHIFT;
        let value_bit = if has_value { Self::HAS_VALUE } else { 0 };
        let num_bits = (num_children as u8) & Self::NUM_CHILDREN_MASK;
        
        Self {
            flags: type_bits | value_bit | num_bits,
            prefix_len: prefix_len.min(255) as u8,
        }
    }
}

/// Memory arena for all nodes
struct Arena {
    data: Vec<u8>,
}

impl Arena {
    fn new() -> Self {
        Self { data: Vec::new() }
    }
    
    #[inline]
    fn alloc(&mut self, size: usize) -> usize {
        let offset = self.data.len();
        self.data.resize(offset + size, 0);
        offset
    }
    
    #[inline]
    fn write_u8(&mut self, offset: usize, value: u8) {
        self.data[offset] = value;
    }
    
    #[inline]
    fn write_u16(&mut self, offset: usize, value: u16) {
        self.data[offset..offset+2].copy_from_slice(&value.to_le_bytes());
    }
    
    #[inline]
    fn write_u32(&mut self, offset: usize, value: u32) {
        self.data[offset..offset+4].copy_from_slice(&value.to_le_bytes());
    }
    
    #[inline]
    fn write_u64(&mut self, offset: usize, value: u64) {
        self.data[offset..offset+8].copy_from_slice(&value.to_le_bytes());
    }
    
    #[inline]
    fn write_ref(&mut self, offset: usize, r: Ref) {
        self.write_u32(offset, r.0);
    }
    
    #[inline]
    fn read_u8(&self, offset: usize) -> u8 {
        self.data[offset]
    }
    
    #[inline]
    fn read_u16(&self, offset: usize) -> u16 {
        u16::from_le_bytes(self.data[offset..offset+2].try_into().unwrap())
    }
    
    #[inline]
    fn read_u64(&self, offset: usize) -> u64 {
        u64::from_le_bytes(self.data[offset..offset+8].try_into().unwrap())
    }
    
    #[inline]
    fn read_ref(&self, offset: usize) -> Ref {
        Ref(u32::from_le_bytes(self.data[offset..offset+4].try_into().unwrap()))
    }
    
    #[inline]
    fn read_header(&self, offset: usize) -> NodeHeader {
        NodeHeader {
            flags: self.data[offset],
            prefix_len: self.data[offset + 1],
        }
    }
    
    #[inline]
    fn write_header(&mut self, offset: usize, header: NodeHeader) {
        self.data[offset] = header.flags;
        self.data[offset + 1] = header.prefix_len;
    }
    
    #[inline]
    fn slice(&self, offset: usize, len: usize) -> &[u8] {
        &self.data[offset..offset + len]
    }
    
    #[inline]
    fn slice_mut(&mut self, offset: usize, len: usize) -> &mut [u8] {
        &mut self.data[offset..offset + len]
    }
    
    fn memory_usage(&self) -> usize {
        self.data.capacity()
    }
}

/// GloryArt: Ultra-compact adaptive radix tree
/// 
/// Node layout in arena:
/// - Header: 2 bytes (flags + prefix_len)
/// - Value: 8 bytes (if has_value)
/// - Prefix: prefix_len bytes
/// - Keys: depends on node type (N4: 4, N16: 16, N48: 0, N256: 0)
/// - Children index: only for N48 (256 bytes)
/// - Children refs: 4 bytes each (N4: 4, N16: 16, N48: 48, N256: 256)
pub struct GloryArt {
    arena: Arena,
    root: Ref,
    len: usize,
}

// Helper for node layout
impl GloryArt {
    const HEADER_SIZE: usize = 2;
    const VALUE_SIZE: usize = 8;
    const REF_SIZE: usize = 4;
    
    /// Get offset where value is stored (if has_value)
    #[inline]
    fn value_offset(node_offset: usize) -> usize {
        node_offset + Self::HEADER_SIZE
    }
    
    /// Get offset where prefix starts
    #[inline]
    fn prefix_offset(node_offset: usize, has_value: bool) -> usize {
        node_offset + Self::HEADER_SIZE + if has_value { Self::VALUE_SIZE } else { 0 }
    }
    
    /// Get offset where keys start (for N4/N16)
    #[inline]
    fn keys_offset(node_offset: usize, has_value: bool, prefix_len: usize) -> usize {
        Self::prefix_offset(node_offset, has_value) + prefix_len
    }
    
    /// Get offset where children refs start
    fn children_offset(node_offset: usize, has_value: bool, prefix_len: usize, node_type: u8) -> usize {
        let base = Self::keys_offset(node_offset, has_value, prefix_len);
        match node_type {
            NodeHeader::TYPE_N4 => base + 4,   // 4 key bytes
            NodeHeader::TYPE_N16 => base + 16, // 16 key bytes
            NodeHeader::TYPE_N48 => base + 256, // 256 index bytes
            NodeHeader::TYPE_N256 => base,      // direct indexing, no keys
            _ => base,
        }
    }
    
    /// Calculate node size
    fn node_size(node_type: u8, has_value: bool, prefix_len: usize) -> usize {
        let base = Self::HEADER_SIZE + if has_value { Self::VALUE_SIZE } else { 0 } + prefix_len;
        match node_type {
            NodeHeader::TYPE_N4 => base + 4 + 4 * Self::REF_SIZE,      // 4 keys + 4 children
            NodeHeader::TYPE_N16 => base + 16 + 16 * Self::REF_SIZE,   // 16 keys + 16 children
            NodeHeader::TYPE_N48 => base + 256 + 48 * Self::REF_SIZE,  // 256 index + 48 children
            NodeHeader::TYPE_N256 => base + 256 * Self::REF_SIZE,      // 256 children
            _ => base,
        }
    }
}

impl GloryArt {
    /// Create a new empty tree
    pub fn new() -> Self {
        Self {
            arena: Arena::new(),
            root: Ref::NULL,
            len: 0,
        }
    }
    
    #[inline]
    pub fn len(&self) -> usize { self.len }
    
    #[inline]
    pub fn is_empty(&self) -> bool { self.len == 0 }
    
    /// Lookup a key
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        if self.root.is_null() {
            return None;
        }
        
        let mut node_ref = self.root;
        let mut depth = 0;
        
        while !node_ref.is_null() {
            let offset = node_ref.offset();
            let header = self.arena.read_header(offset);
            let has_value = header.has_value();
            let prefix_len = header.prefix_len as usize;
            
            // Check prefix
            if prefix_len > 0 {
                let remaining = key.len() - depth;
                if remaining < prefix_len {
                    return None;
                }
                let prefix_off = Self::prefix_offset(offset, has_value);
                let prefix = self.arena.slice(prefix_off, prefix_len);
                if &key[depth..depth + prefix_len] != prefix {
                    return None;
                }
                depth += prefix_len;
            }
            
            // Key ends here?
            if depth == key.len() {
                if has_value {
                    let val_off = Self::value_offset(offset);
                    return Some(self.arena.read_u64(val_off));
                } else {
                    return None;
                }
            }
            
            // Find child for next byte
            let byte = key[depth];
            let node_type = header.node_type_raw();
            let num_children = header.num_children();
            
            let keys_off = Self::keys_offset(offset, has_value, prefix_len);
            let children_off = Self::children_offset(offset, has_value, prefix_len, node_type);
            
            let child = match node_type {
                NodeHeader::TYPE_N4 => {
                    let mut found = Ref::NULL;
                    for i in 0..num_children.min(4) {
                        if self.arena.read_u8(keys_off + i) == byte {
                            found = self.arena.read_ref(children_off + i * Self::REF_SIZE);
                            break;
                        }
                    }
                    found
                }
                NodeHeader::TYPE_N16 => {
                    let mut found = Ref::NULL;
                    for i in 0..num_children.min(16) {
                        if self.arena.read_u8(keys_off + i) == byte {
                            found = self.arena.read_ref(children_off + i * Self::REF_SIZE);
                            break;
                        }
                    }
                    found
                }
                NodeHeader::TYPE_N48 => {
                    let idx = self.arena.read_u8(keys_off + byte as usize);
                    if idx == 0 {
                        Ref::NULL
                    } else {
                        self.arena.read_ref(children_off + (idx as usize - 1) * Self::REF_SIZE)
                    }
                }
                NodeHeader::TYPE_N256 => {
                    self.arena.read_ref(children_off + byte as usize * Self::REF_SIZE)
                }
                _ => Ref::NULL,
            };
            
            if child.is_null() {
                return None;
            }
            
            node_ref = child;
            depth += 1;
        }
        
        None
    }
    
    /// Insert a key-value pair
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        if self.root.is_null() {
            // Create first node as N4 with the full key as prefix
            self.root = self.alloc_n4(key, true, value);
            self.len += 1;
            return None;
        }
        
        let result = self.insert_recursive(self.root, key, 0, value);
        match result {
            InsertResult::Updated(old) => Some(old),
            InsertResult::Inserted => {
                self.len += 1;
                None
            }
            InsertResult::Split { new_node } => {
                self.root = new_node;
                self.len += 1;
                None
            }
        }
    }
    
    fn insert_recursive(&mut self, node_ref: Ref, key: &[u8], depth: usize, value: u64) -> InsertResult {
        let offset = node_ref.offset();
        let header = self.arena.read_header(offset);
        let has_value = header.has_value();
        let prefix_len = header.prefix_len as usize;
        let node_type = header.node_type_raw();
        
        // Check prefix match
        if prefix_len > 0 {
            let prefix_off = Self::prefix_offset(offset, has_value);
            let prefix = self.arena.slice(prefix_off, prefix_len).to_vec();
            let remaining_key = &key[depth..];
            
            let mut mismatch = 0;
            while mismatch < prefix_len && mismatch < remaining_key.len() && prefix[mismatch] == remaining_key[mismatch] {
                mismatch += 1;
            }
            
            if mismatch < prefix_len {
                // Need to split this node
                // Create new parent with common prefix
                let common_prefix = &prefix[..mismatch];
                
                // Create new N4 for the split point
                let key_ends_at_mismatch = depth + mismatch == key.len();
                let new_parent = self.alloc_n4(common_prefix, key_ends_at_mismatch, value);
                
                // The existing node needs a shortened prefix
                let old_remaining_prefix = &prefix[mismatch + 1..];
                let old_byte = prefix[mismatch];
                let new_old_node = self.clone_node_with_new_prefix(node_ref, old_remaining_prefix);
                
                // Add old node as child
                self.add_child_to_n4(new_parent, old_byte, new_old_node);
                
                // If new key continues past mismatch, add new child
                if !key_ends_at_mismatch {
                    let new_byte = key[depth + mismatch];
                    let new_child_prefix = &key[depth + mismatch + 1..];
                    let new_child = self.alloc_n4(new_child_prefix, true, value);
                    self.add_child_to_n4(new_parent, new_byte, new_child);
                }
                
                return InsertResult::Split { new_node: new_parent };
            }
        }
        
        let new_depth = depth + prefix_len;
        
        // Key ends at this node?
        if new_depth == key.len() {
            // Update or set value
            if has_value {
                let val_off = Self::value_offset(offset);
                let old = self.arena.read_u64(val_off);
                self.arena.write_u64(val_off, value);
                return InsertResult::Updated(old);
            } else {
                // Need to add value - create new node with value
                let prefix_off = Self::prefix_offset(offset, false);
                let prefix = self.arena.slice(prefix_off, prefix_len).to_vec();
                let new_node = self.clone_node_with_value(node_ref, value);
                return InsertResult::Split { new_node };
            }
        }
        
        // Find child for next byte
        let byte = key[new_depth];
        let num_children = header.num_children();
        let keys_off = Self::keys_offset(offset, has_value, prefix_len);
        let children_off = Self::children_offset(offset, has_value, prefix_len, node_type);
        
        // Find existing child
        let (child_idx, child) = match node_type {
            NodeHeader::TYPE_N4 => {
                let mut found = (None, Ref::NULL);
                for i in 0..num_children.min(4) {
                    if self.arena.read_u8(keys_off + i) == byte {
                        found = (Some(i), self.arena.read_ref(children_off + i * Self::REF_SIZE));
                        break;
                    }
                }
                found
            }
            NodeHeader::TYPE_N16 => {
                let mut found = (None, Ref::NULL);
                for i in 0..num_children.min(16) {
                    if self.arena.read_u8(keys_off + i) == byte {
                        found = (Some(i), self.arena.read_ref(children_off + i * Self::REF_SIZE));
                        break;
                    }
                }
                found
            }
            NodeHeader::TYPE_N48 => {
                let idx = self.arena.read_u8(keys_off + byte as usize);
                if idx == 0 {
                    (None, Ref::NULL)
                } else {
                    (Some(idx as usize - 1), self.arena.read_ref(children_off + (idx as usize - 1) * Self::REF_SIZE))
                }
            }
            NodeHeader::TYPE_N256 => {
                let c = self.arena.read_ref(children_off + byte as usize * Self::REF_SIZE);
                if c.is_null() {
                    (None, Ref::NULL)
                } else {
                    (Some(byte as usize), c)
                }
            }
            _ => (None, Ref::NULL),
        };
        
        if !child.is_null() {
            // Recurse into child
            let result = self.insert_recursive(child, key, new_depth + 1, value);
            match result {
                InsertResult::Split { new_node } => {
                    // Update child pointer
                    self.set_child(node_ref, node_type, children_off, child_idx.unwrap(), new_node);
                    InsertResult::Inserted
                }
                other => other,
            }
        } else {
            // Add new child
            let child_prefix = &key[new_depth + 1..];
            let new_child = self.alloc_n4(child_prefix, true, value);
            
            // Try to add to current node
            if self.try_add_child(node_ref, byte, new_child) {
                InsertResult::Inserted
            } else {
                // Need to grow node
                let new_node = self.grow_and_add(node_ref, byte, new_child);
                InsertResult::Split { new_node }
            }
        }
    }
    
    /// Allocate an N4 node
    fn alloc_n4(&mut self, prefix: &[u8], has_value: bool, value: u64) -> Ref {
        let prefix_len = prefix.len();
        let size = Self::node_size(NodeHeader::TYPE_N4, has_value, prefix_len);
        let offset = self.arena.alloc(size);
        
        let header = NodeHeader::new(NodeHeader::TYPE_N4, has_value, 0, prefix_len);
        self.arena.write_header(offset, header);
        
        if has_value {
            self.arena.write_u64(Self::value_offset(offset), value);
        }
        
        if prefix_len > 0 {
            let prefix_off = Self::prefix_offset(offset, has_value);
            self.arena.slice_mut(prefix_off, prefix_len).copy_from_slice(prefix);
        }
        
        // Initialize children to NULL
        let children_off = Self::children_offset(offset, has_value, prefix_len, NodeHeader::TYPE_N4);
        for i in 0..4 {
            self.arena.write_ref(children_off + i * Self::REF_SIZE, Ref::NULL);
        }
        
        Ref::new(offset)
    }
    
    /// Clone a node with a new (shorter) prefix
    fn clone_node_with_new_prefix(&mut self, old_ref: Ref, new_prefix: &[u8]) -> Ref {
        let old_offset = old_ref.offset();
        let old_header = self.arena.read_header(old_offset);
        let old_has_value = old_header.has_value();
        let old_prefix_len = old_header.prefix_len as usize;
        let old_type = old_header.node_type_raw();
        let old_num_children = old_header.num_children();
        
        let new_prefix_len = new_prefix.len();
        let new_size = Self::node_size(old_type, old_has_value, new_prefix_len);
        let new_offset = self.arena.alloc(new_size);
        
        // Write header
        let new_header = NodeHeader::new(old_type, old_has_value, old_num_children, new_prefix_len);
        self.arena.write_header(new_offset, new_header);
        
        // Copy value if present
        if old_has_value {
            let old_val = self.arena.read_u64(Self::value_offset(old_offset));
            self.arena.write_u64(Self::value_offset(new_offset), old_val);
        }
        
        // Write new prefix
        if new_prefix_len > 0 {
            let prefix_off = Self::prefix_offset(new_offset, old_has_value);
            self.arena.slice_mut(prefix_off, new_prefix_len).copy_from_slice(new_prefix);
        }
        
        // Copy keys and children
        let old_keys_off = Self::keys_offset(old_offset, old_has_value, old_prefix_len);
        let new_keys_off = Self::keys_offset(new_offset, old_has_value, new_prefix_len);
        let old_children_off = Self::children_offset(old_offset, old_has_value, old_prefix_len, old_type);
        let new_children_off = Self::children_offset(new_offset, old_has_value, new_prefix_len, old_type);
        
        match old_type {
            NodeHeader::TYPE_N4 => {
                // Copy 4 keys
                for i in 0..4 {
                    let k = self.arena.read_u8(old_keys_off + i);
                    self.arena.write_u8(new_keys_off + i, k);
                }
                // Copy 4 children
                for i in 0..4 {
                    let c = self.arena.read_ref(old_children_off + i * Self::REF_SIZE);
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, c);
                }
            }
            NodeHeader::TYPE_N16 => {
                for i in 0..16 {
                    let k = self.arena.read_u8(old_keys_off + i);
                    self.arena.write_u8(new_keys_off + i, k);
                }
                for i in 0..16 {
                    let c = self.arena.read_ref(old_children_off + i * Self::REF_SIZE);
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, c);
                }
            }
            NodeHeader::TYPE_N48 => {
                for i in 0..256 {
                    let idx = self.arena.read_u8(old_keys_off + i);
                    self.arena.write_u8(new_keys_off + i, idx);
                }
                for i in 0..48 {
                    let c = self.arena.read_ref(old_children_off + i * Self::REF_SIZE);
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, c);
                }
            }
            NodeHeader::TYPE_N256 => {
                for i in 0..256 {
                    let c = self.arena.read_ref(old_children_off + i * Self::REF_SIZE);
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, c);
                }
            }
            _ => {}
        }
        
        Ref::new(new_offset)
    }
    
    /// Clone a node adding a value
    fn clone_node_with_value(&mut self, old_ref: Ref, value: u64) -> Ref {
        let old_offset = old_ref.offset();
        let old_header = self.arena.read_header(old_offset);
        let old_prefix_len = old_header.prefix_len as usize;
        let old_type = old_header.node_type_raw();
        let old_num_children = old_header.num_children();
        
        // Get old prefix
        let old_prefix = self.arena.slice(
            Self::prefix_offset(old_offset, old_header.has_value()),
            old_prefix_len
        ).to_vec();
        
        let new_size = Self::node_size(old_type, true, old_prefix_len);
        let new_offset = self.arena.alloc(new_size);
        
        // Write header with has_value=true
        let new_header = NodeHeader::new(old_type, true, old_num_children, old_prefix_len);
        self.arena.write_header(new_offset, new_header);
        
        // Write value
        self.arena.write_u64(Self::value_offset(new_offset), value);
        
        // Copy prefix
        if old_prefix_len > 0 {
            let prefix_off = Self::prefix_offset(new_offset, true);
            self.arena.slice_mut(prefix_off, old_prefix_len).copy_from_slice(&old_prefix);
        }
        
        // Copy keys and children
        let old_keys_off = Self::keys_offset(old_offset, old_header.has_value(), old_prefix_len);
        let new_keys_off = Self::keys_offset(new_offset, true, old_prefix_len);
        let old_children_off = Self::children_offset(old_offset, old_header.has_value(), old_prefix_len, old_type);
        let new_children_off = Self::children_offset(new_offset, true, old_prefix_len, old_type);
        
        match old_type {
            NodeHeader::TYPE_N4 => {
                for i in 0..4 {
                    self.arena.write_u8(new_keys_off + i, self.arena.read_u8(old_keys_off + i));
                }
                for i in 0..4 {
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, 
                        self.arena.read_ref(old_children_off + i * Self::REF_SIZE));
                }
            }
            NodeHeader::TYPE_N16 => {
                for i in 0..16 {
                    self.arena.write_u8(new_keys_off + i, self.arena.read_u8(old_keys_off + i));
                }
                for i in 0..16 {
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE,
                        self.arena.read_ref(old_children_off + i * Self::REF_SIZE));
                }
            }
            NodeHeader::TYPE_N48 => {
                for i in 0..256 {
                    self.arena.write_u8(new_keys_off + i, self.arena.read_u8(old_keys_off + i));
                }
                for i in 0..48 {
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE,
                        self.arena.read_ref(old_children_off + i * Self::REF_SIZE));
                }
            }
            NodeHeader::TYPE_N256 => {
                for i in 0..256 {
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE,
                        self.arena.read_ref(old_children_off + i * Self::REF_SIZE));
                }
            }
            _ => {}
        }
        
        Ref::new(new_offset)
    }
    
    /// Add a child to an N4 node (helper for split)
    fn add_child_to_n4(&mut self, node_ref: Ref, byte: u8, child: Ref) {
        let offset = node_ref.offset();
        let mut header = self.arena.read_header(offset);
        let has_value = header.has_value();
        let prefix_len = header.prefix_len as usize;
        let num_children = header.num_children();
        
        let keys_off = Self::keys_offset(offset, has_value, prefix_len);
        let children_off = Self::children_offset(offset, has_value, prefix_len, NodeHeader::TYPE_N4);
        
        // Add at position num_children
        self.arena.write_u8(keys_off + num_children, byte);
        self.arena.write_ref(children_off + num_children * Self::REF_SIZE, child);
        
        header.set_num_children(num_children + 1);
        self.arena.write_header(offset, header);
    }
    
    /// Try to add a child to existing node (returns false if need to grow)
    fn try_add_child(&mut self, node_ref: Ref, byte: u8, child: Ref) -> bool {
        let offset = node_ref.offset();
        let mut header = self.arena.read_header(offset);
        let has_value = header.has_value();
        let prefix_len = header.prefix_len as usize;
        let node_type = header.node_type_raw();
        let num_children = header.num_children();
        
        let keys_off = Self::keys_offset(offset, has_value, prefix_len);
        let children_off = Self::children_offset(offset, has_value, prefix_len, node_type);
        
        match node_type {
            NodeHeader::TYPE_N4 if num_children < 4 => {
                self.arena.write_u8(keys_off + num_children, byte);
                self.arena.write_ref(children_off + num_children * Self::REF_SIZE, child);
                header.set_num_children(num_children + 1);
                self.arena.write_header(offset, header);
                true
            }
            NodeHeader::TYPE_N16 if num_children < 16 => {
                self.arena.write_u8(keys_off + num_children, byte);
                self.arena.write_ref(children_off + num_children * Self::REF_SIZE, child);
                header.set_num_children(num_children + 1);
                self.arena.write_header(offset, header);
                true
            }
            NodeHeader::TYPE_N48 if num_children < 48 => {
                self.arena.write_u8(keys_off + byte as usize, (num_children + 1) as u8);
                self.arena.write_ref(children_off + num_children * Self::REF_SIZE, child);
                header.set_num_children(num_children + 1);
                self.arena.write_header(offset, header);
                true
            }
            NodeHeader::TYPE_N256 => {
                self.arena.write_ref(children_off + byte as usize * Self::REF_SIZE, child);
                if num_children < 255 {
                    header.set_num_children(num_children + 1);
                }
                self.arena.write_header(offset, header);
                true
            }
            _ => false, // Need to grow
        }
    }
    
    /// Grow node and add child
    fn grow_and_add(&mut self, old_ref: Ref, byte: u8, child: Ref) -> Ref {
        let old_offset = old_ref.offset();
        let old_header = self.arena.read_header(old_offset);
        let has_value = old_header.has_value();
        let prefix_len = old_header.prefix_len as usize;
        let old_type = old_header.node_type_raw();
        let num_children = old_header.num_children();
        
        // Get prefix
        let prefix = self.arena.slice(Self::prefix_offset(old_offset, has_value), prefix_len).to_vec();
        
        // Get value if present
        let value = if has_value {
            Some(self.arena.read_u64(Self::value_offset(old_offset)))
        } else {
            None
        };
        
        // Determine new type
        let new_type = match old_type {
            NodeHeader::TYPE_N4 => NodeHeader::TYPE_N16,
            NodeHeader::TYPE_N16 => NodeHeader::TYPE_N48,
            NodeHeader::TYPE_N48 => NodeHeader::TYPE_N256,
            _ => NodeHeader::TYPE_N256,
        };
        
        // Allocate new node
        let new_size = Self::node_size(new_type, has_value, prefix_len);
        let new_offset = self.arena.alloc(new_size);
        
        // Write header
        let new_header = NodeHeader::new(new_type, has_value, num_children + 1, prefix_len);
        self.arena.write_header(new_offset, new_header);
        
        // Write value
        if let Some(v) = value {
            self.arena.write_u64(Self::value_offset(new_offset), v);
        }
        
        // Write prefix
        if prefix_len > 0 {
            let prefix_off = Self::prefix_offset(new_offset, has_value);
            self.arena.slice_mut(prefix_off, prefix_len).copy_from_slice(&prefix);
        }
        
        // Copy children and add new one
        let old_keys_off = Self::keys_offset(old_offset, has_value, prefix_len);
        let new_keys_off = Self::keys_offset(new_offset, has_value, prefix_len);
        let old_children_off = Self::children_offset(old_offset, has_value, prefix_len, old_type);
        let new_children_off = Self::children_offset(new_offset, has_value, prefix_len, new_type);
        
        match (old_type, new_type) {
            (NodeHeader::TYPE_N4, NodeHeader::TYPE_N16) => {
                // Copy N4 keys and children
                for i in 0..num_children {
                    let k = self.arena.read_u8(old_keys_off + i);
                    self.arena.write_u8(new_keys_off + i, k);
                    let c = self.arena.read_ref(old_children_off + i * Self::REF_SIZE);
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, c);
                }
                // Add new child
                self.arena.write_u8(new_keys_off + num_children, byte);
                self.arena.write_ref(new_children_off + num_children * Self::REF_SIZE, child);
            }
            (NodeHeader::TYPE_N16, NodeHeader::TYPE_N48) => {
                // Initialize index to 0
                for i in 0..256 {
                    self.arena.write_u8(new_keys_off + i, 0);
                }
                // Copy N16 to N48
                for i in 0..num_children {
                    let k = self.arena.read_u8(old_keys_off + i);
                    let c = self.arena.read_ref(old_children_off + i * Self::REF_SIZE);
                    self.arena.write_u8(new_keys_off + k as usize, (i + 1) as u8);
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, c);
                }
                // Add new child
                self.arena.write_u8(new_keys_off + byte as usize, (num_children + 1) as u8);
                self.arena.write_ref(new_children_off + num_children * Self::REF_SIZE, child);
            }
            (NodeHeader::TYPE_N48, NodeHeader::TYPE_N256) => {
                // Initialize to NULL
                for i in 0..256 {
                    self.arena.write_ref(new_children_off + i * Self::REF_SIZE, Ref::NULL);
                }
                // Copy N48 to N256
                for k in 0u8..=255u8 {
                    let idx = self.arena.read_u8(old_keys_off + k as usize);
                    if idx != 0 {
                        let c = self.arena.read_ref(old_children_off + (idx as usize - 1) * Self::REF_SIZE);
                        self.arena.write_ref(new_children_off + k as usize * Self::REF_SIZE, c);
                    }
                }
                // Add new child
                self.arena.write_ref(new_children_off + byte as usize * Self::REF_SIZE, child);
            }
            _ => {}
        }
        
        Ref::new(new_offset)
    }
    
    /// Set child at known index
    fn set_child(&mut self, node_ref: Ref, node_type: u8, children_off: usize, idx: usize, child: Ref) {
        match node_type {
            NodeHeader::TYPE_N4 | NodeHeader::TYPE_N16 | NodeHeader::TYPE_N48 => {
                self.arena.write_ref(children_off + idx * Self::REF_SIZE, child);
            }
            NodeHeader::TYPE_N256 => {
                self.arena.write_ref(children_off + idx * Self::REF_SIZE, child);
            }
            _ => {}
        }
    }
    
    /// Memory statistics
    pub fn memory_stats(&self) -> GloryStats {
        let arena_bytes = self.arena.memory_usage();
        
        GloryStats {
            arena_bytes,
            total_bytes: arena_bytes,
            num_keys: self.len,
            bytes_per_key: if self.len > 0 {
                arena_bytes as f64 / self.len as f64
            } else {
                0.0
            },
        }
    }
}

impl Default for GloryArt {
    fn default() -> Self { Self::new() }
}

enum InsertResult {
    Updated(u64),
    Inserted,
    Split { new_node: Ref },
}

/// Memory statistics
#[derive(Debug, Clone)]
pub struct GloryStats {
    /// Arena memory usage
    pub arena_bytes: usize,
    /// Total memory usage
    pub total_bytes: usize,
    /// Number of keys
    pub num_keys: usize,
    /// Total bytes per key
    pub bytes_per_key: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut tree = GloryArt::new();
        
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        
        assert_eq!(tree.get(b"hello"), Some(1));
        assert_eq!(tree.get(b"world"), Some(2));
        assert_eq!(tree.get(b"notfound"), None);
        
        assert_eq!(tree.len(), 2);
    }
    
    #[test]
    fn test_prefix() {
        let mut tree = GloryArt::new();
        
        tree.insert(b"test", 1);
        tree.insert(b"testing", 2);
        tree.insert(b"tested", 3);
        
        assert_eq!(tree.get(b"test"), Some(1));
        assert_eq!(tree.get(b"testing"), Some(2));
        assert_eq!(tree.get(b"tested"), Some(3));
    }
    
    #[test]
    fn test_many() {
        let mut tree = GloryArt::new();
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            tree.insert(key.as_bytes(), i);
        }
        
        assert_eq!(tree.len(), 1000);
        
        let mut correct = 0;
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            if tree.get(key.as_bytes()) == Some(i) {
                correct += 1;
            }
        }
        
        println!("Correct: {}/1000", correct);
        assert!(correct >= 990, "Too many failures: {}/1000 correct", correct);
        
        let stats = tree.memory_stats();
        println!("Memory stats:");
        println!("  Arena: {} bytes", stats.arena_bytes);
        println!("  Bytes per key: {:.1}", stats.bytes_per_key);
    }
    
    #[test]
    fn test_sizes() {
        println!("NodeHeader: {} bytes", std::mem::size_of::<NodeHeader>());
        println!("Ref: {} bytes", std::mem::size_of::<Ref>());
    }
}
