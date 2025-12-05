//! Fast ART implementation inspired by libart (C)
//!
//! Key optimizations from libart:
//! 1. Pointer tagging - low bit distinguishes leaf vs internal node
//! 2. Inline key storage - keys embedded in leaf allocations
//! 3. Compact node headers - only 16 bytes
//! 4. SSE2 for Node16 lookup

#![allow(unsafe_op_in_unsafe_fn)]

use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;

/// Maximum prefix length stored inline
const MAX_PREFIX: usize = 10;

/// Tagged pointer: low bit = 1 means leaf, 0 means internal node
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct TaggedPtr(usize);

impl TaggedPtr {
    #[inline]
    pub const fn null() -> Self {
        Self(0)
    }
    
    #[inline]
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
    
    #[inline]
    pub fn is_leaf(self) -> bool {
        (self.0 & 1) != 0
    }
    
    #[inline]
    pub fn from_leaf(leaf: NonNull<Leaf>) -> Self {
        Self(leaf.as_ptr() as usize | 1)
    }
    
    #[inline]
    pub fn from_node<T>(node: NonNull<T>) -> Self {
        Self(node.as_ptr() as usize)
    }
    
    #[inline]
    pub unsafe fn as_leaf(self) -> NonNull<Leaf> {
        debug_assert!(self.is_leaf() && !self.is_null());
        NonNull::new_unchecked((self.0 & !1) as *mut Leaf)
    }
    
    #[inline]
    pub unsafe fn as_node(self) -> NonNull<NodeHeader> {
        debug_assert!(!self.is_leaf() && !self.is_null());
        NonNull::new_unchecked(self.0 as *mut NodeHeader)
    }
}

/// Node types
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum NodeType {
    Node4 = 1,
    Node16 = 2,
    Node48 = 3,
    Node256 = 4,
}

/// Common node header (16 bytes, matching libart)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct NodeHeader {
    pub partial_len: u32,
    pub node_type: NodeType,
    pub num_children: u8,
    pub partial: [u8; MAX_PREFIX],
}

/// Node4: up to 4 children
#[repr(C)]
pub struct Node4 {
    pub header: NodeHeader,
    pub keys: [u8; 4],
    pub children: [TaggedPtr; 4],
}

/// Node16: up to 16 children
#[repr(C)]
pub struct Node16 {
    pub header: NodeHeader,
    pub keys: [u8; 16],
    pub children: [TaggedPtr; 16],
}

/// Node48: up to 48 children with 256-byte index
#[repr(C)]
pub struct Node48 {
    pub header: NodeHeader,
    pub keys: [u8; 256],
    pub children: [TaggedPtr; 48],
}

/// Node256: up to 256 children
#[repr(C)]
pub struct Node256 {
    pub header: NodeHeader,
    pub children: [TaggedPtr; 256],
}

/// Leaf node with inline key
#[repr(C)]
pub struct Leaf {
    pub value: u64,
    pub key_len: u32,
    // Key bytes follow immediately after
}

impl Leaf {
    pub fn alloc(key: &[u8], value: u64) -> NonNull<Leaf> {
        let layout = Self::layout(key.len());
        unsafe {
            let ptr = alloc(layout) as *mut Leaf;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            (*ptr).value = value;
            (*ptr).key_len = key.len() as u32;
            let key_ptr = (ptr as *mut u8).add(std::mem::size_of::<Leaf>());
            std::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
            NonNull::new_unchecked(ptr)
        }
    }
    
    #[inline]
    pub fn key(&self) -> &[u8] {
        unsafe {
            let key_ptr = (self as *const Leaf as *const u8).add(std::mem::size_of::<Leaf>());
            std::slice::from_raw_parts(key_ptr, self.key_len as usize)
        }
    }
    
    pub unsafe fn free(ptr: NonNull<Leaf>) {
        let key_len = (*ptr.as_ptr()).key_len as usize;
        let layout = Self::layout(key_len);
        dealloc(ptr.as_ptr() as *mut u8, layout);
    }
    
    fn layout(key_len: usize) -> Layout {
        Layout::from_size_align(
            std::mem::size_of::<Leaf>() + key_len,
            std::mem::align_of::<Leaf>(),
        ).unwrap()
    }
    
    #[inline]
    pub fn matches(&self, key: &[u8]) -> bool {
        // Keys are stored with terminating byte, so compare directly
        self.key() == key
    }
    
    /// Get the original key (without terminating byte)
    #[inline]
    pub fn original_key(&self) -> &[u8] {
        let k = self.key();
        if k.is_empty() { k } else { &k[..k.len() - 1] }
    }
}

impl Node4 {
    pub fn alloc() -> NonNull<Node4> {
        let layout = Layout::new::<Node4>();
        unsafe {
            let ptr = alloc(layout) as *mut Node4;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            std::ptr::write_bytes(ptr, 0, 1);
            (*ptr).header.node_type = NodeType::Node4;
            NonNull::new_unchecked(ptr)
        }
    }
    
    #[inline]
    pub fn find_child(&self, byte: u8) -> Option<TaggedPtr> {
        let n = self.header.num_children as usize;
        for i in 0..n {
            if self.keys[i] == byte {
                return Some(self.children[i]);
            }
        }
        None
    }
    
    #[inline]
    pub fn find_child_idx(&self, byte: u8) -> Option<usize> {
        let n = self.header.num_children as usize;
        for i in 0..n {
            if self.keys[i] == byte {
                return Some(i);
            }
        }
        None
    }
    
    #[inline]
    pub fn add_child(&mut self, byte: u8, child: TaggedPtr) {
        debug_assert!((self.header.num_children as usize) < 4);
        let n = self.header.num_children as usize;
        
        let mut pos = n;
        for i in 0..n {
            if byte < self.keys[i] {
                pos = i;
                break;
            }
        }
        
        for i in (pos..n).rev() {
            self.keys[i + 1] = self.keys[i];
            self.children[i + 1] = self.children[i];
        }
        
        self.keys[pos] = byte;
        self.children[pos] = child;
        self.header.num_children += 1;
    }
    
    pub unsafe fn free(ptr: NonNull<Node4>) {
        dealloc(ptr.as_ptr() as *mut u8, Layout::new::<Node4>());
    }
}

impl Node16 {
    pub fn alloc() -> NonNull<Node16> {
        let layout = Layout::new::<Node16>();
        unsafe {
            let ptr = alloc(layout) as *mut Node16;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            std::ptr::write_bytes(ptr, 0, 1);
            (*ptr).header.node_type = NodeType::Node16;
            NonNull::new_unchecked(ptr)
        }
    }
    
    /// Find child using SIMD when available
    #[inline]
    pub fn find_child(&self, byte: u8) -> Option<TaggedPtr> {
        #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
        {
            return self.find_child_simd(byte);
        }
        
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sse2")))]
        {
            self.find_child_scalar(byte)
        }
    }
    
    /// SIMD-optimized child lookup using SSE2
    #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
    #[inline]
    fn find_child_simd(&self, byte: u8) -> Option<TaggedPtr> {
        use std::arch::x86_64::*;
        
        let n = self.header.num_children as usize;
        if n == 0 {
            return None;
        }
        
        unsafe {
            // Load 16 bytes of keys
            let keys_vec = _mm_loadu_si128(self.keys.as_ptr() as *const __m128i);
            // Create vector of search byte
            let search = _mm_set1_epi8(byte as i8);
            // Compare equal
            let cmp = _mm_cmpeq_epi8(keys_vec, search);
            // Get mask of matching positions
            let mask = _mm_movemask_epi8(cmp) as u32;
            
            // Mask off invalid positions (beyond num_children)
            let valid_mask = (1u32 << n) - 1;
            let result = mask & valid_mask;
            
            if result != 0 {
                // Return first matching child
                let idx = result.trailing_zeros() as usize;
                Some(self.children[idx])
            } else {
                None
            }
        }
    }
    
    /// Scalar fallback for finding child
    #[inline]
    fn find_child_scalar(&self, byte: u8) -> Option<TaggedPtr> {
        let n = self.header.num_children as usize;
        for i in 0..n {
            if self.keys[i] == byte {
                return Some(self.children[i]);
            }
        }
        None
    }
    
    /// Find child index using SIMD when available
    #[inline]
    pub fn find_child_idx(&self, byte: u8) -> Option<usize> {
        #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
        {
            return self.find_child_idx_simd(byte);
        }
        
        #[cfg(not(all(target_arch = "x86_64", target_feature = "sse2")))]
        {
            self.find_child_idx_scalar(byte)
        }
    }
    
    /// SIMD-optimized child index lookup
    #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
    #[inline]
    fn find_child_idx_simd(&self, byte: u8) -> Option<usize> {
        use std::arch::x86_64::*;
        
        let n = self.header.num_children as usize;
        if n == 0 {
            return None;
        }
        
        unsafe {
            let keys_vec = _mm_loadu_si128(self.keys.as_ptr() as *const __m128i);
            let search = _mm_set1_epi8(byte as i8);
            let cmp = _mm_cmpeq_epi8(keys_vec, search);
            let mask = _mm_movemask_epi8(cmp) as u32;
            
            let valid_mask = (1u32 << n) - 1;
            let result = mask & valid_mask;
            
            if result != 0 {
                Some(result.trailing_zeros() as usize)
            } else {
                None
            }
        }
    }
    
    /// Scalar fallback for finding child index
    #[inline]
    fn find_child_idx_scalar(&self, byte: u8) -> Option<usize> {
        let n = self.header.num_children as usize;
        for i in 0..n {
            if self.keys[i] == byte {
                return Some(i);
            }
        }
        None
    }
    
    #[inline]
    pub fn add_child(&mut self, byte: u8, child: TaggedPtr) {
        debug_assert!((self.header.num_children as usize) < 16);
        let n = self.header.num_children as usize;
        
        let mut pos = n;
        for i in 0..n {
            if byte < self.keys[i] {
                pos = i;
                break;
            }
        }
        
        for i in (pos..n).rev() {
            self.keys[i + 1] = self.keys[i];
            self.children[i + 1] = self.children[i];
        }
        
        self.keys[pos] = byte;
        self.children[pos] = child;
        self.header.num_children += 1;
    }
    
    pub unsafe fn free(ptr: NonNull<Node16>) {
        dealloc(ptr.as_ptr() as *mut u8, Layout::new::<Node16>());
    }
}

impl Node48 {
    pub fn alloc() -> NonNull<Node48> {
        let layout = Layout::new::<Node48>();
        unsafe {
            let ptr = alloc(layout) as *mut Node48;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            std::ptr::write_bytes(ptr, 0, 1);
            (*ptr).header.node_type = NodeType::Node48;
            NonNull::new_unchecked(ptr)
        }
    }
    
    #[inline]
    pub fn find_child(&self, byte: u8) -> Option<TaggedPtr> {
        let idx = self.keys[byte as usize];
        if idx == 0 {
            None
        } else {
            Some(self.children[(idx - 1) as usize])
        }
    }
    
    #[inline]
    pub fn add_child(&mut self, byte: u8, child: TaggedPtr) {
        debug_assert!((self.header.num_children as usize) < 48);
        let slot = self.header.num_children as usize;
        self.children[slot] = child;
        self.keys[byte as usize] = (slot + 1) as u8;
        self.header.num_children += 1;
    }
    
    pub unsafe fn free(ptr: NonNull<Node48>) {
        dealloc(ptr.as_ptr() as *mut u8, Layout::new::<Node48>());
    }
}

impl Node256 {
    pub fn alloc() -> NonNull<Node256> {
        let layout = Layout::new::<Node256>();
        unsafe {
            let ptr = alloc(layout) as *mut Node256;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            std::ptr::write_bytes(ptr, 0, 1);
            (*ptr).header.node_type = NodeType::Node256;
            NonNull::new_unchecked(ptr)
        }
    }
    
    #[inline]
    pub fn find_child(&self, byte: u8) -> Option<TaggedPtr> {
        let child = self.children[byte as usize];
        if child.is_null() {
            None
        } else {
            Some(child)
        }
    }
    
    #[inline]
    pub fn add_child(&mut self, byte: u8, child: TaggedPtr) {
        if self.children[byte as usize].is_null() {
            self.header.num_children += 1;
        }
        self.children[byte as usize] = child;
    }
    
    pub unsafe fn free(ptr: NonNull<Node256>) {
        dealloc(ptr.as_ptr() as *mut u8, Layout::new::<Node256>());
    }
}

/// Fast ART with libart-style optimizations
pub struct FastArt {
    root: TaggedPtr,
    size: usize,
}

impl FastArt {
    pub fn new() -> Self {
        Self {
            root: TaggedPtr::null(),
            size: 0,
        }
    }
    
    #[inline]
    pub fn len(&self) -> usize {
        self.size
    }
    
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
    
    pub fn get(&self, key: &[u8]) -> Option<u64> {
        // Add terminating byte like libart
        let mut key_with_term = Vec::with_capacity(key.len() + 1);
        key_with_term.extend_from_slice(key);
        key_with_term.push(0);
        unsafe { self.search(&key_with_term) }
    }
    
    unsafe fn search(&self, key: &[u8]) -> Option<u64> {
        let mut node = self.root;
        let mut depth = 0;
        
        while !node.is_null() {
            if node.is_leaf() {
                let leaf = node.as_leaf().as_ref();
                if leaf.matches(key) {
                    return Some(leaf.value);
                }
                return None;
            }
            
            let header = &*(node.as_node().as_ptr() as *const NodeHeader);
            let prefix_len = header.partial_len as usize;
            
            // Check prefix
            if prefix_len > 0 {
                let check_len = prefix_len.min(MAX_PREFIX);
                if depth + check_len > key.len() {
                    return None;
                }
                for i in 0..check_len {
                    if header.partial[i] != key[depth + i] {
                        return None;
                    }
                }
                
                // For long prefixes, verify against leaf
                if prefix_len > MAX_PREFIX {
                    let min_leaf = Self::minimum_static(node);
                    if let Some(leaf) = min_leaf {
                        let leaf_key = (*leaf.as_ptr()).key();
                        for i in MAX_PREFIX..prefix_len {
                            if depth + i >= key.len() || depth + i >= leaf_key.len() {
                                return None;
                            }
                            if leaf_key[depth + i] != key[depth + i] {
                                return None;
                            }
                        }
                    }
                }
                
                depth += prefix_len;
            }
            
            if depth >= key.len() {
                return None;
            }
            
            let byte = key[depth];
            let child = match header.node_type {
                NodeType::Node4 => {
                    let n = &*(node.as_node().as_ptr() as *const Node4);
                    n.find_child(byte)
                }
                NodeType::Node16 => {
                    let n = &*(node.as_node().as_ptr() as *const Node16);
                    n.find_child(byte)
                }
                NodeType::Node48 => {
                    let n = &*(node.as_node().as_ptr() as *const Node48);
                    n.find_child(byte)
                }
                NodeType::Node256 => {
                    let n = &*(node.as_node().as_ptr() as *const Node256);
                    n.find_child(byte)
                }
            };
            
            match child {
                Some(c) => {
                    node = c;
                    depth += 1;
                }
                None => return None,
            }
        }
        
        None
    }
    
    unsafe fn minimum_static(node: TaggedPtr) -> Option<NonNull<Leaf>> {
        if node.is_null() {
            return None;
        }
        if node.is_leaf() {
            return Some(node.as_leaf());
        }
        
        let header = &*(node.as_node().as_ptr() as *const NodeHeader);
        match header.node_type {
            NodeType::Node4 => {
                let n = &*(node.as_node().as_ptr() as *const Node4);
                if n.header.num_children > 0 {
                    Self::minimum_static(n.children[0])
                } else {
                    None
                }
            }
            NodeType::Node16 => {
                let n = &*(node.as_node().as_ptr() as *const Node16);
                if n.header.num_children > 0 {
                    Self::minimum_static(n.children[0])
                } else {
                    None
                }
            }
            NodeType::Node48 => {
                let n = &*(node.as_node().as_ptr() as *const Node48);
                for byte in 0..256 {
                    let idx = n.keys[byte];
                    if idx != 0 {
                        return Self::minimum_static(n.children[(idx - 1) as usize]);
                    }
                }
                None
            }
            NodeType::Node256 => {
                let n = &*(node.as_node().as_ptr() as *const Node256);
                for i in 0..256 {
                    if !n.children[i].is_null() {
                        return Self::minimum_static(n.children[i]);
                    }
                }
                None
            }
        }
    }
    
    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        // Add terminating byte like libart
        let mut key_with_term = Vec::with_capacity(key.len() + 1);
        key_with_term.extend_from_slice(key);
        key_with_term.push(0);
        
        if self.root.is_null() {
            let leaf = Leaf::alloc(&key_with_term, value);
            self.root = TaggedPtr::from_leaf(leaf);
            self.size += 1;
            return None;
        }
        
        let root_ptr = &mut self.root as *mut TaggedPtr;
        let result = unsafe { 
            Self::recursive_insert_static(root_ptr, &key_with_term, 0, value)
        };
        if result.is_none() {
            self.size += 1;
        }
        result
    }
    
    unsafe fn recursive_insert_static(
        node_ptr: *mut TaggedPtr,
        key: &[u8],
        depth: usize,
        value: u64,
    ) -> Option<u64> {
        let node = *node_ptr;
        
        // Leaf node
        if node.is_leaf() {
            let leaf = node.as_leaf();
            let existing_key = (*leaf.as_ptr()).key();
            
            // Update existing
            if existing_key == key {
                let old = (*leaf.as_ptr()).value;
                (*leaf.as_ptr()).value = value;
                return Some(old);
            }
            
            // Split: create new node
            let new_node = Node4::alloc();
            let new_ptr = new_node.as_ptr();
            
            // Find longest common prefix starting from depth
            let mut prefix_len = 0;
            let max_check = (key.len() - depth).min(existing_key.len() - depth);
            while prefix_len < max_check 
                && key[depth + prefix_len] == existing_key[depth + prefix_len] 
            {
                prefix_len += 1;
            }
            
            (*new_ptr).header.partial_len = prefix_len as u32;
            let copy_len = prefix_len.min(MAX_PREFIX);
            for i in 0..copy_len {
                (*new_ptr).header.partial[i] = key[depth + i];
            }
            
            // Add existing leaf
            if depth + prefix_len < existing_key.len() {
                (*new_ptr).add_child(existing_key[depth + prefix_len], node);
            }
            
            // Add new leaf
            let new_leaf = Leaf::alloc(key, value);
            if depth + prefix_len < key.len() {
                (*new_ptr).add_child(key[depth + prefix_len], TaggedPtr::from_leaf(new_leaf));
            }
            
            *node_ptr = TaggedPtr::from_node(new_node);
            return None;
        }
        
        // Internal node
        let header_ptr = node.as_node().as_ptr() as *mut NodeHeader;
        let prefix_len = (*header_ptr).partial_len as usize;
        
        if prefix_len > 0 {
            // Check for prefix mismatch
            let mismatch = Self::prefix_mismatch_static(node, key, depth);
            
            if mismatch < prefix_len {
                // Split node at mismatch point
                let new_node = Node4::alloc();
                let new_ptr = new_node.as_ptr();
                
                (*new_ptr).header.partial_len = mismatch as u32;
                let copy_len = mismatch.min(MAX_PREFIX);
                for i in 0..copy_len {
                    (*new_ptr).header.partial[i] = (*header_ptr).partial[i];
                }
                
                // Adjust old node's prefix
                if prefix_len <= MAX_PREFIX {
                    (*new_ptr).add_child((*header_ptr).partial[mismatch], node);
                    let new_prefix_len = prefix_len - mismatch - 1;
                    (*header_ptr).partial_len = new_prefix_len as u32;
                    // Shift prefix
                    for i in 0..new_prefix_len.min(MAX_PREFIX) {
                        (*header_ptr).partial[i] = (*header_ptr).partial[mismatch + 1 + i];
                    }
                } else {
                    // Long prefix - need to get byte from leaf
                    let min_leaf = Self::minimum_static(node).unwrap();
                    let leaf_key = (*min_leaf.as_ptr()).key();
                    (*new_ptr).add_child(leaf_key[depth + mismatch], node);
                    let new_prefix_len = prefix_len - mismatch - 1;
                    (*header_ptr).partial_len = new_prefix_len as u32;
                    let copy_start = depth + mismatch + 1;
                    let copy_len = new_prefix_len.min(MAX_PREFIX);
                    for i in 0..copy_len {
                        (*header_ptr).partial[i] = leaf_key[copy_start + i];
                    }
                }
                
                // Add new leaf
                let new_leaf = Leaf::alloc(key, value);
                (*new_ptr).add_child(key[depth + mismatch], TaggedPtr::from_leaf(new_leaf));
                
                *node_ptr = TaggedPtr::from_node(new_node);
                return None;
            }
        }
        
        let new_depth = depth + prefix_len;
        
        if new_depth >= key.len() {
            // Key exhausted at internal node - would need combined leaf+node
            return None;
        }
        
        let byte = key[new_depth];
        
        // Find child or add new one
        match (*header_ptr).node_type {
            NodeType::Node4 => {
                let n = node.as_node().as_ptr() as *mut Node4;
                if let Some(idx) = (*n).find_child_idx(byte) {
                    let child_ptr = &mut (*n).children[idx] as *mut TaggedPtr;
                    return Self::recursive_insert_static(child_ptr, key, new_depth + 1, value);
                }
                if ((*n).header.num_children as usize) < 4 {
                    let new_leaf = Leaf::alloc(key, value);
                    (*n).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                    return None;
                }
                // Grow
                let new_node = Self::grow_node4_static(n);
                *node_ptr = TaggedPtr::from_node(new_node);
                let new_leaf = Leaf::alloc(key, value);
                (*new_node.as_ptr()).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                None
            }
            NodeType::Node16 => {
                let n = node.as_node().as_ptr() as *mut Node16;
                if let Some(idx) = (*n).find_child_idx(byte) {
                    let child_ptr = &mut (*n).children[idx] as *mut TaggedPtr;
                    return Self::recursive_insert_static(child_ptr, key, new_depth + 1, value);
                }
                if ((*n).header.num_children as usize) < 16 {
                    let new_leaf = Leaf::alloc(key, value);
                    (*n).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                    return None;
                }
                let new_node = Self::grow_node16_static(n);
                *node_ptr = TaggedPtr::from_node(new_node);
                let new_leaf = Leaf::alloc(key, value);
                (*new_node.as_ptr()).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                None
            }
            NodeType::Node48 => {
                let n = node.as_node().as_ptr() as *mut Node48;
                let idx = (*n).keys[byte as usize];
                if idx != 0 {
                    let child_ptr = &mut (*n).children[(idx - 1) as usize] as *mut TaggedPtr;
                    return Self::recursive_insert_static(child_ptr, key, new_depth + 1, value);
                }
                if ((*n).header.num_children as usize) < 48 {
                    let new_leaf = Leaf::alloc(key, value);
                    (*n).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                    return None;
                }
                let new_node = Self::grow_node48_static(n);
                *node_ptr = TaggedPtr::from_node(new_node);
                let new_leaf = Leaf::alloc(key, value);
                (*new_node.as_ptr()).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                None
            }
            NodeType::Node256 => {
                let n = node.as_node().as_ptr() as *mut Node256;
                if !(*n).children[byte as usize].is_null() {
                    let child_ptr = &mut (*n).children[byte as usize] as *mut TaggedPtr;
                    return Self::recursive_insert_static(child_ptr, key, new_depth + 1, value);
                }
                let new_leaf = Leaf::alloc(key, value);
                (*n).add_child(byte, TaggedPtr::from_leaf(new_leaf));
                None
            }
        }
    }
    
    unsafe fn prefix_mismatch_static(node: TaggedPtr, key: &[u8], depth: usize) -> usize {
        let header = &*(node.as_node().as_ptr() as *const NodeHeader);
        let prefix_len = header.partial_len as usize;
        
        let check_len = prefix_len.min(MAX_PREFIX).min(key.len() - depth);
        for i in 0..check_len {
            if header.partial[i] != key[depth + i] {
                return i;
            }
        }
        
        // If prefix is longer than MAX_PREFIX, check against leaf
        if prefix_len > MAX_PREFIX {
            if let Some(min_leaf) = Self::minimum_static(node) {
                let leaf_key = (*min_leaf.as_ptr()).key();
                for i in MAX_PREFIX..prefix_len {
                    if depth + i >= key.len() || depth + i >= leaf_key.len() {
                        return i;
                    }
                    if leaf_key[depth + i] != key[depth + i] {
                        return i;
                    }
                }
            }
        }
        
        prefix_len
    }
    
    unsafe fn grow_node4_static(node: *mut Node4) -> NonNull<Node16> {
        let new_node = Node16::alloc();
        let new_ptr = new_node.as_ptr();
        
        (*new_ptr).header = (*node).header;
        (*new_ptr).header.node_type = NodeType::Node16;
        
        let n = (*node).header.num_children as usize;
        for i in 0..n {
            (*new_ptr).keys[i] = (*node).keys[i];
            (*new_ptr).children[i] = (*node).children[i];
        }
        
        Node4::free(NonNull::new_unchecked(node));
        new_node
    }
    
    unsafe fn grow_node16_static(node: *mut Node16) -> NonNull<Node48> {
        let new_node = Node48::alloc();
        let new_ptr = new_node.as_ptr();
        
        (*new_ptr).header = (*node).header;
        (*new_ptr).header.node_type = NodeType::Node48;
        
        let n = (*node).header.num_children as usize;
        for i in 0..n {
            (*new_ptr).children[i] = (*node).children[i];
            (*new_ptr).keys[(*node).keys[i] as usize] = (i + 1) as u8;
        }
        
        Node16::free(NonNull::new_unchecked(node));
        new_node
    }
    
    unsafe fn grow_node48_static(node: *mut Node48) -> NonNull<Node256> {
        let new_node = Node256::alloc();
        let new_ptr = new_node.as_ptr();
        
        (*new_ptr).header = (*node).header;
        (*new_ptr).header.node_type = NodeType::Node256;
        
        for byte in 0..256 {
            let idx = (*node).keys[byte];
            if idx != 0 {
                (*new_ptr).children[byte] = (*node).children[(idx - 1) as usize];
            }
        }
        
        Node48::free(NonNull::new_unchecked(node));
        new_node
    }
    
    unsafe fn free_recursive(node: TaggedPtr) {
        if node.is_null() {
            return;
        }
        
        if node.is_leaf() {
            Leaf::free(node.as_leaf());
            return;
        }
        
        let header = &*(node.as_node().as_ptr() as *const NodeHeader);
        match header.node_type {
            NodeType::Node4 => {
                let n = &*(node.as_node().as_ptr() as *const Node4);
                for i in 0..n.header.num_children as usize {
                    Self::free_recursive(n.children[i]);
                }
                Node4::free(NonNull::new_unchecked(node.as_node().as_ptr() as *mut Node4));
            }
            NodeType::Node16 => {
                let n = &*(node.as_node().as_ptr() as *const Node16);
                for i in 0..n.header.num_children as usize {
                    Self::free_recursive(n.children[i]);
                }
                Node16::free(NonNull::new_unchecked(node.as_node().as_ptr() as *mut Node16));
            }
            NodeType::Node48 => {
                let n = &*(node.as_node().as_ptr() as *const Node48);
                for byte in 0..256 {
                    let idx = n.keys[byte];
                    if idx != 0 {
                        Self::free_recursive(n.children[(idx - 1) as usize]);
                    }
                }
                Node48::free(NonNull::new_unchecked(node.as_node().as_ptr() as *mut Node48));
            }
            NodeType::Node256 => {
                let n = &*(node.as_node().as_ptr() as *const Node256);
                for child in n.children.iter() {
                    if !child.is_null() {
                        Self::free_recursive(*child);
                    }
                }
                Node256::free(NonNull::new_unchecked(node.as_node().as_ptr() as *mut Node256));
            }
        }
    }
}

impl Default for FastArt {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FastArt {
    fn drop(&mut self) {
        if !self.root.is_null() {
            unsafe { Self::free_recursive(self.root); }
        }
    }
}

unsafe impl Send for FastArt {}
unsafe impl Sync for FastArt {}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_basic() {
        let mut art = FastArt::new();
        
        assert!(art.insert(b"hello", 1).is_none());
        assert!(art.insert(b"world", 2).is_none());
        
        assert_eq!(art.get(b"hello"), Some(1));
        assert_eq!(art.get(b"world"), Some(2));
        assert_eq!(art.get(b"notfound"), None);
        
        assert_eq!(art.insert(b"hello", 3), Some(1));
        assert_eq!(art.get(b"hello"), Some(3));
        
        assert_eq!(art.len(), 2);
    }
    
    #[test]
    fn test_prefix_sharing() {
        let mut art = FastArt::new();
        
        art.insert(b"test", 1);
        art.insert(b"testing", 2);
        art.insert(b"tested", 3);
        
        assert_eq!(art.get(b"test"), Some(1));
        assert_eq!(art.get(b"testing"), Some(2));
        assert_eq!(art.get(b"tested"), Some(3));
    }
    
    #[test]
    fn test_many() {
        let mut art = FastArt::new();
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            art.insert(key.as_bytes(), i);
        }
        
        assert_eq!(art.len(), 1000);
        
        for i in 0..1000u64 {
            let key = format!("key{:04}", i);
            assert_eq!(art.get(key.as_bytes()), Some(i), "Failed for key {}", i);
        }
    }
    
    #[test]
    fn test_node_sizes() {
        println!("TaggedPtr: {} bytes", std::mem::size_of::<TaggedPtr>());
        println!("NodeHeader: {} bytes", std::mem::size_of::<NodeHeader>());
        println!("Node4: {} bytes", std::mem::size_of::<Node4>());
        println!("Node16: {} bytes", std::mem::size_of::<Node16>());
        println!("Node48: {} bytes", std::mem::size_of::<Node48>());
        println!("Node256: {} bytes", std::mem::size_of::<Node256>());
        println!("Leaf: {} bytes (+ key)", std::mem::size_of::<Leaf>());
        
        assert_eq!(std::mem::size_of::<NodeHeader>(), 16);
    }
}
