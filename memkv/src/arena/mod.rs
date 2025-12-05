//! Memory arena for efficient allocation.
//!
//! Arenas provide contiguous memory allocation with minimal overhead.
//! This is crucial for memory efficiency as it:
//! - Eliminates per-allocation overhead
//! - Improves cache locality
//! - Enables pointer compression (32-bit offsets instead of 64-bit pointers)
//! - Simplifies memory tracking

use std::cell::UnsafeCell;
use std::mem;
use std::ptr::NonNull;

/// Default chunk size for arena allocation (1MB)
const DEFAULT_CHUNK_SIZE: usize = 1024 * 1024;

/// Minimum alignment for allocations
const MIN_ALIGN: usize = 8;

/// A memory arena for efficient bulk allocation.
///
/// The arena allocates memory in large chunks and hands out slices.
/// Individual allocations cannot be freed - only the entire arena can be dropped.
pub struct Arena {
    chunks: UnsafeCell<Vec<Box<[u8]>>>,
    current: UnsafeCell<*mut u8>,
    remaining: UnsafeCell<usize>,
    chunk_size: usize,
    total_allocated: UnsafeCell<usize>,
}

impl Arena {
    /// Create a new arena with default chunk size (1MB).
    pub fn new() -> Self {
        Self::with_chunk_size(DEFAULT_CHUNK_SIZE)
    }

    /// Create a new arena with a specific chunk size.
    pub fn with_chunk_size(chunk_size: usize) -> Self {
        Self {
            chunks: UnsafeCell::new(Vec::new()),
            current: UnsafeCell::new(std::ptr::null_mut()),
            remaining: UnsafeCell::new(0),
            chunk_size,
            total_allocated: UnsafeCell::new(0),
        }
    }

    /// Allocate `size` bytes with the given alignment.
    ///
    /// # Safety
    /// The returned pointer is valid for the lifetime of the arena.
    pub fn alloc(&self, size: usize, align: usize) -> NonNull<u8> {
        let align = align.max(MIN_ALIGN);
        
        unsafe {
            let remaining = &mut *self.remaining.get();
            let current = &mut *self.current.get();
            
            // Align the current pointer
            let aligned = (*current as usize + align - 1) & !(align - 1);
            let padding = aligned - *current as usize;
            let total_size = size + padding;

            if total_size > *remaining {
                self.grow(size.max(self.chunk_size), align);
                return self.alloc(size, align);
            }

            let ptr = aligned as *mut u8;
            *current = ptr.add(size);
            *remaining -= total_size;

            NonNull::new_unchecked(ptr)
        }
    }

    /// Allocate and copy bytes into the arena.
    pub fn alloc_bytes(&self, bytes: &[u8]) -> NonNull<[u8]> {
        if bytes.is_empty() {
            // Return a dangling but aligned pointer for empty slices
            return NonNull::slice_from_raw_parts(NonNull::dangling(), 0);
        }
        
        let ptr = self.alloc(bytes.len(), 1);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
            NonNull::slice_from_raw_parts(ptr, bytes.len())
        }
    }

    /// Allocate space for a value and move it into the arena.
    pub fn alloc_value<T>(&self, value: T) -> NonNull<T> {
        let ptr = self.alloc(mem::size_of::<T>(), mem::align_of::<T>());
        let typed_ptr = ptr.as_ptr() as *mut T;
        unsafe {
            typed_ptr.write(value);
            NonNull::new_unchecked(typed_ptr)
        }
    }

    /// Get the total bytes allocated by this arena.
    pub fn total_allocated(&self) -> usize {
        unsafe { *self.total_allocated.get() }
    }

    /// Get the number of chunks allocated.
    pub fn num_chunks(&self) -> usize {
        unsafe { (*self.chunks.get()).len() }
    }

    /// Grow the arena by allocating a new chunk.
    fn grow(&self, min_size: usize, _align: usize) {
        let size = min_size.max(self.chunk_size);
        let chunk: Box<[u8]> = vec![0u8; size].into_boxed_slice();
        
        unsafe {
            let chunks = &mut *self.chunks.get();
            let total = &mut *self.total_allocated.get();
            let current = &mut *self.current.get();
            let remaining = &mut *self.remaining.get();

            *total += size;
            *current = chunk.as_ptr() as *mut u8;
            *remaining = size;
            chunks.push(chunk);
        }
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: Arena allocations are valid for the arena's lifetime
// and we only hand out raw pointers
unsafe impl Send for Arena {}
unsafe impl Sync for Arena {}

/// A 32-bit offset into an arena, used for pointer compression.
///
/// This reduces pointer size from 8 bytes to 4 bytes, but limits
/// the maximum arena size to 4GB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct ArenaOffset(u32);

impl ArenaOffset {
    /// Create a null offset (represents no value).
    pub const fn null() -> Self {
        Self(u32::MAX)
    }

    /// Check if this offset is null.
    pub fn is_null(self) -> bool {
        self.0 == u32::MAX
    }

    /// Create an offset from a usize.
    ///
    /// # Panics
    /// Panics if the offset is >= 2^32 - 1.
    pub fn from_usize(offset: usize) -> Self {
        assert!(offset < u32::MAX as usize, "Arena offset too large");
        Self(offset as u32)
    }

    /// Get the offset as usize.
    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// An arena that supports offset-based access for pointer compression.
pub struct OffsetArena {
    data: UnsafeCell<Vec<u8>>,
}

impl OffsetArena {
    /// Create a new offset arena with the given initial capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            data: UnsafeCell::new(Vec::with_capacity(capacity)),
        }
    }

    /// Allocate bytes and return an offset.
    pub fn alloc_bytes(&self, bytes: &[u8]) -> ArenaOffset {
        if bytes.is_empty() {
            return ArenaOffset::null();
        }
        
        unsafe {
            let data = &mut *self.data.get();
            let offset = data.len();
            data.extend_from_slice(bytes);
            ArenaOffset::from_usize(offset)
        }
    }

    /// Get bytes at an offset.
    ///
    /// # Safety
    /// The offset must have been returned by `alloc_bytes` on this arena,
    /// and `len` must be the length that was originally allocated.
    pub unsafe fn get_bytes(&self, offset: ArenaOffset, len: usize) -> &[u8] {
        if offset.is_null() {
            return &[];
        }
        let data = unsafe { &*self.data.get() };
        &data[offset.as_usize()..offset.as_usize() + len]
    }

    /// Get total bytes used.
    pub fn len(&self) -> usize {
        unsafe { (*self.data.get()).len() }
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get capacity.
    pub fn capacity(&self) -> usize {
        unsafe { (*self.data.get()).capacity() }
    }
}

impl Default for OffsetArena {
    fn default() -> Self {
        Self::new(4096)
    }
}

unsafe impl Send for OffsetArena {}
unsafe impl Sync for OffsetArena {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arena_basic() {
        let arena = Arena::new();
        
        let ptr1 = arena.alloc_bytes(b"hello");
        let ptr2 = arena.alloc_bytes(b"world");
        
        unsafe {
            assert_eq!(&*ptr1.as_ptr(), b"hello");
            assert_eq!(&*ptr2.as_ptr(), b"world");
        }
    }

    #[test]
    fn test_arena_value() {
        let arena = Arena::new();
        
        let ptr = arena.alloc_value(42u64);
        unsafe {
            assert_eq!(*ptr.as_ptr(), 42);
        }
    }

    #[test]
    fn test_offset_arena() {
        let arena = OffsetArena::new(1024);
        
        let off1 = arena.alloc_bytes(b"hello");
        let off2 = arena.alloc_bytes(b"world");
        
        unsafe {
            assert_eq!(arena.get_bytes(off1, 5), b"hello");
            assert_eq!(arena.get_bytes(off2, 5), b"world");
        }
    }

    #[test]
    fn test_arena_empty_bytes() {
        let arena = OffsetArena::new(1024);
        let off = arena.alloc_bytes(b"");
        assert!(off.is_null());
    }
}
