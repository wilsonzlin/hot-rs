//! FFI bindings to libart (C implementation of ART)
//! 
//! This provides a comparison baseline using the original C implementation.

use std::ffi::c_void;
use std::os::raw::{c_int, c_uchar};

#[repr(C)]
pub struct ArtTree {
    root: *mut c_void,
    size: u64,
}

extern "C" {
    pub fn art_tree_init(t: *mut ArtTree) -> c_int;
    pub fn art_tree_destroy(t: *mut ArtTree) -> c_int;
    pub fn art_insert(
        t: *mut ArtTree,
        key: *const c_uchar,
        key_len: c_int,
        value: *mut c_void,
    ) -> *mut c_void;
    pub fn art_search(
        t: *const ArtTree,
        key: *const c_uchar,
        key_len: c_int,
    ) -> *mut c_void;
    pub fn art_delete(
        t: *mut ArtTree,
        key: *const c_uchar,
        key_len: c_int,
    ) -> *mut c_void;
}

/// Safe wrapper around libart
pub struct LibArt {
    tree: ArtTree,
}

impl LibArt {
    pub fn new() -> Self {
        let mut tree = ArtTree {
            root: std::ptr::null_mut(),
            size: 0,
        };
        unsafe {
            art_tree_init(&mut tree);
        }
        Self { tree }
    }

    pub fn insert(&mut self, key: &[u8], value: u64) -> Option<u64> {
        let old = unsafe {
            art_insert(
                &mut self.tree,
                key.as_ptr(),
                key.len() as c_int,
                value as *mut c_void,
            )
        };
        if old.is_null() {
            None
        } else {
            Some(old as u64)
        }
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        let result = unsafe {
            art_search(&self.tree, key.as_ptr(), key.len() as c_int)
        };
        if result.is_null() {
            None
        } else {
            Some(result as u64)
        }
    }

    pub fn len(&self) -> usize {
        self.tree.size as usize
    }

    pub fn is_empty(&self) -> bool {
        self.tree.size == 0
    }
}

impl Drop for LibArt {
    fn drop(&mut self) {
        unsafe {
            art_tree_destroy(&mut self.tree);
        }
    }
}

impl Default for LibArt {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: LibArt is not thread-safe (use mutex if needed)
unsafe impl Send for LibArt {}
