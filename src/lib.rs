//! # hot-rs
//!
//! A memory-efficient ordered map using Height Optimized Trie (HOT).
//!
//! Based on "HOT: A Height Optimized Trie Index for Main-Memory Database Systems"
//! (SIGMOD 2018, Binna et al.)
//!
//! ## Example
//!
//! ```rust
//! use hot_rs::HotTree;
//!
//! let mut tree: HotTree<u64> = HotTree::new();
//! tree.insert(b"hello", 1);
//! tree.insert(b"world", 2);
//!
//! assert_eq!(tree.get(b"hello"), Some(&1));
//! assert_eq!(tree.get(b"world"), Some(&2));
//! ```

#![deny(unsafe_op_in_unsafe_fn)]

use std::collections::HashMap;
use std::marker::PhantomData;

#[derive(Clone, Copy)]
struct InsertFrame {
    node_off: u64,
    /// Entry index (0..n-1) followed while descending.
    entry_idx: usize,
    /// Most significant discriminative bit index (root BiNode discriminator).
    msb: u16,
}

#[derive(Clone, Copy)]
struct BiNodeSplit {
    disc: u16,
    height: u8,
    left: Ptr,
    right: Ptr,
}

// =============================================================================
// Configuration
// =============================================================================

const MIN_PREFIX_LEN: usize = 4; // Minimum prefix length to consider
const MAX_PREFIX_LEN: usize = 128; // Maximum prefix length
const MAX_PREFIXES: usize = 65535; // Maximum unique prefixes (u16 max - 1)

// =============================================================================
// Bit utilities (PEXT helpers + bit indexing)
// =============================================================================

#[inline]
fn bit_byte_index(bit: u16) -> u16 {
    bit / 8
}

/// Bit index within its byte, where `0` is the MSB and `7` is the LSB.
#[inline]
fn bit_in_byte_msb0(bit: u16) -> u8 {
    (bit % 8) as u8
}

#[inline]
fn prefix_mask_before_bit_in_byte(bit_in_byte_msb0: u8) -> u8 {
    debug_assert!(bit_in_byte_msb0 <= 7);
    if bit_in_byte_msb0 == 0 {
        0
    } else {
        0xFFu8 << (8 - bit_in_byte_msb0)
    }
}

#[inline]
fn pext_u64_fallback(value: u64, mut mask: u64) -> u64 {
    let mut out = 0u64;
    let mut out_bit = 1u64;
    while mask != 0 {
        let lsb = mask & mask.wrapping_neg();
        if (value & lsb) != 0 {
            out |= out_bit;
        }
        mask ^= lsb;
        out_bit <<= 1;
    }
    out
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
unsafe fn pext_u64_bmi2(value: u64, mask: u64) -> u64 {
    // SAFETY: Caller guarantees BMI2.
    core::arch::x86_64::_pext_u64(value, mask)
}

#[inline]
fn pext_u64(value: u64, mask: u64) -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("bmi2") {
            // SAFETY: feature detected at runtime.
            return unsafe { pext_u64_bmi2(value, mask) };
        }
    }
    pext_u64_fallback(value, mask)
}

// =============================================================================
// Pointer type
// =============================================================================

/// Pointer: 40-bit tagged (stored in `u64`).
///
/// Layout (little endian in storage, but treated as an integer here):
/// - Bit 39 = 1: leaf (byte offset into `leaves`)
/// - Bit 39 = 0: node (byte offset into `nodes`)
/// - Bit 38 = 1: tombstone (only meaningful for leaf pointers when `V` is ZST)
/// - Special: 0xFF_FFFF_FFFF (40-bit all 1s) = NULL
#[derive(Clone, Copy, PartialEq, Eq)]
struct Ptr(u64);

impl Ptr {
    const LEAF_BIT: u64 = 1u64 << 39;
    const TOMBSTONE_BIT: u64 = 1u64 << 38;
    const OFFSET_MASK: u64 = (1u64 << 38) - 1;
    const NULL: Ptr = Ptr((1u64 << 40) - 1);

    #[inline]
    fn leaf(off: u64) -> Self {
        debug_assert!(off <= Self::OFFSET_MASK);
        Self(off | Self::LEAF_BIT)
    }

    #[inline]
    fn node(off: u64) -> Self {
        debug_assert!(off <= Self::OFFSET_MASK);
        Self(off)
    }

    #[inline]
    fn is_null(self) -> bool {
        self.0 == Self::NULL.0
    }

    #[inline]
    fn is_leaf(self) -> bool {
        !self.is_null() && (self.0 & Self::LEAF_BIT) != 0
    }

    #[inline]
    fn is_tombstone(self) -> bool {
        self.is_leaf() && (self.0 & Self::TOMBSTONE_BIT) != 0
    }

    #[inline]
    fn without_tombstone(self) -> Self {
        debug_assert!(self.is_leaf());
        Self(self.0 & !Self::TOMBSTONE_BIT)
    }

    #[inline]
    fn offset(self) -> u64 {
        self.0 & Self::OFFSET_MASK
    }

    #[inline]
    fn leaf_off(self) -> u64 {
        debug_assert!(self.is_leaf());
        self.offset()
    }

    #[inline]
    fn node_off(self) -> u64 {
        debug_assert!(!self.is_null() && !self.is_leaf());
        self.offset()
    }
}

// =============================================================================
// HOT discriminative-bits representations (SingleMask / MultiMask)
// =============================================================================

#[derive(Clone, Copy, Debug)]
struct SingleMaskPartialKeyMapping {
    most_significant_bit: u16,
    least_significant_bit: u16,
    /// Byte offset for the 8-byte window used for extraction.
    byte_offset: u16,
    /// Extraction mask over the 64-bit big-endian window.
    extraction_mask: u64,
}

impl SingleMaskPartialKeyMapping {
    #[inline]
    fn num_bits(self) -> u16 {
        self.extraction_mask.count_ones() as u16
    }

    fn discriminative_bits(self, out: &mut Vec<u16>) {
        let mut mask = self.extraction_mask;
        while mask != 0 {
            let lsb = mask & mask.wrapping_neg();
            let bit_pos = lsb.trailing_zeros() as u32; // 0..63, where 0 is LSB
            mask ^= lsb;

            let idx_from_msb = 63u32 - bit_pos;
            let rel_byte = (idx_from_msb / 8) as u16;
            let rel_bit_msb0 = (idx_from_msb % 8) as u16;
            out.push((self.byte_offset + rel_byte) * 8 + rel_bit_msb0);
        }
    }

    fn try_from_bits(discriminative_bits: &[u16]) -> Option<Self> {
        debug_assert!(!discriminative_bits.is_empty());

        let mut msb = u16::MAX;
        let mut lsb = 0u16;
        for &b in discriminative_bits {
            msb = msb.min(b);
            lsb = lsb.max(b);
        }

        let msb_byte = bit_byte_index(msb);
        let lsb_byte = bit_byte_index(lsb);
        if lsb_byte.saturating_sub(msb_byte) > 7 {
            return None;
        }

        let byte_offset = lsb_byte.saturating_sub(7);
        let mut extraction_mask = 0u64;
        for &bit in discriminative_bits {
            let byte = bit_byte_index(bit);
            let rel_byte = byte - byte_offset;
            debug_assert!(rel_byte <= 7);
            let rel_bit_msb0 = bit_in_byte_msb0(bit) as u64;
            let bit_index = 63u64 - (u64::from(rel_byte) * 8 + rel_bit_msb0);
            extraction_mask |= 1u64 << bit_index;
        }

        Some(Self {
            most_significant_bit: msb,
            least_significant_bit: lsb,
            byte_offset,
            extraction_mask,
        })
    }

    #[inline]
    fn extract_u32(self, key: &[u8]) -> u32 {
        let off = self.byte_offset as usize;
        let mut bytes = [0u8; 8];
        for i in 0..8 {
            bytes[i] = key.get(off + i).copied().unwrap_or(0);
        }
        let word = u64::from_be_bytes(bytes);
        pext_u64(word, self.extraction_mask) as u32
    }

    #[inline]
    fn prefix_mask_u32(self, discriminative_bit: u16) -> u32 {
        let disc_byte = bit_byte_index(discriminative_bit);
        let disc_bit_msb0 = bit_in_byte_msb0(discriminative_bit);

        let mut bytes = [0u8; 8];
        for i in 0..8 {
            let abs_byte = self.byte_offset + i as u16;
            bytes[i] = if abs_byte < disc_byte {
                0xFF
            } else if abs_byte == disc_byte {
                prefix_mask_before_bit_in_byte(disc_bit_msb0)
            } else {
                0
            };
        }

        let word = u64::from_be_bytes(bytes);
        pext_u64(word, self.extraction_mask) as u32
    }
}

#[derive(Clone, Copy, Debug)]
struct MultiMaskPartialKeyMapping<const N: usize> {
    most_significant_bit: u16,
    least_significant_bit: u16,
    /// Base byte offset for each extraction group (each group uses up to 8 bytes).
    ///
    /// This extends the original HOT reference implementation (which assumes keys
    /// fit within 256 bytes) to support larger keys by allowing each group to use
    /// its own base byte.
    base_bytes: [u16; N],
    /// Number of used extraction bytes (<= 8*N). Unused slots must have mask byte = 0.
    used_bytes: u8,
    /// For each group: 8 byte positions relative to `base_bytes[g]` (big-endian byte order).
    positions_be: [u64; N],
    /// For each group: 8 byte masks (big-endian) selecting bits within each extracted byte.
    masks_be: [u64; N],
}

impl<const N: usize> MultiMaskPartialKeyMapping<N> {
    #[inline]
    fn num_bits(self) -> u16 {
        let mut bits = 0u16;
        for &m in &self.masks_be {
            bits = bits.saturating_add(m.count_ones() as u16);
        }
        bits
    }

    fn discriminative_bits(self, out: &mut Vec<u16>) {
        for g in 0..N {
            let pos = self.positions_be[g].to_be_bytes();
            let masks = self.masks_be[g].to_be_bytes();
            for i in 0..8 {
                let mask_byte = masks[i];
                if mask_byte == 0 {
                    continue;
                }
                let abs_byte = self.base_bytes[g] + u16::from(pos[i]);
                for bit_msb0 in 0u16..8 {
                    let bit_mask = 0x80u8 >> bit_msb0;
                    if (mask_byte & bit_mask) != 0 {
                        out.push(abs_byte * 8 + bit_msb0);
                    }
                }
            }
        }
    }

    fn try_from_bits(discriminative_bits: &[u16]) -> Option<Self> {
        debug_assert!(!discriminative_bits.is_empty());

        // Gather distinct bytes + per-byte mask.
        let mut byte_pos: [u16; 32] = [0; 32];
        let mut byte_mask: [u8; 32] = [0; 32];
        let mut bytes_len = 0usize;

        for &bit in discriminative_bits {
            let b = bit_byte_index(bit);
            let bit_msb0 = bit_in_byte_msb0(bit);
            let entry_mask = 0x80u8 >> bit_msb0;

            let mut found = None;
            for i in 0..bytes_len {
                if byte_pos[i] == b {
                    found = Some(i);
                    break;
                }
            }
            match found {
                Some(i) => byte_mask[i] |= entry_mask,
                None => {
                    if bytes_len >= 32 {
                        return None;
                    }
                    byte_pos[bytes_len] = b;
                    byte_mask[bytes_len] = entry_mask;
                    bytes_len += 1;
                }
            }
        }

        // Sort by byte position (insertion sort, bytes_len <= 32).
        for i in 1..bytes_len {
            let mut j = i;
            while j > 0 && byte_pos[j - 1] > byte_pos[j] {
                byte_pos.swap(j - 1, j);
                byte_mask.swap(j - 1, j);
                j -= 1;
            }
        }

        if bytes_len > 8 * N {
            return None;
        }

        let msb = *discriminative_bits.iter().min().unwrap();
        let lsb = *discriminative_bits.iter().max().unwrap();

        let mut base_bytes: [u16; N] = [0; N];
        let mut positions_bytes = [[0u8; 8]; N];
        let mut masks_bytes = [[0u8; 8]; N];

        // Partition into up to N groups, each containing <=8 bytes and spanning <=255 bytes.
        let mut g = 0usize;
        let mut idx = 0usize;
        let mut base = byte_pos[0];
        base_bytes[0] = base;
        for i in 0..bytes_len {
            let b = byte_pos[i];
            if idx == 8 || (b - base) > 255 {
                g += 1;
                if g >= N {
                    return None;
                }
                base = b;
                base_bytes[g] = base;
                idx = 0;
            }
            positions_bytes[g][idx] = (b - base) as u8;
            masks_bytes[g][idx] = byte_mask[i];
            idx += 1;
        }

        let mut positions_be = [0u64; N];
        let mut masks_be = [0u64; N];
        for g in 0..N {
            positions_be[g] = u64::from_be_bytes(positions_bytes[g]);
            masks_be[g] = u64::from_be_bytes(masks_bytes[g]);
        }

        Some(Self {
            most_significant_bit: msb,
            least_significant_bit: lsb,
            base_bytes,
            used_bytes: bytes_len as u8,
            positions_be,
            masks_be,
        })
    }

    #[inline]
    fn extract_u32(self, key: &[u8]) -> u32 {
        let mut out = 0u32;
        for g in 0..N {
            let pos = self.positions_be[g].to_be_bytes();
            let masks = self.masks_be[g];
            if masks == 0 {
                continue;
            }

            let mut gathered = [0u8; 8];
            for i in 0..8 {
                let abs_byte = self.base_bytes[g] + u16::from(pos[i]);
                gathered[i] = key.get(abs_byte as usize).copied().unwrap_or(0);
            }
            let word = u64::from_be_bytes(gathered);
            let part = pext_u64(word, masks) as u32;
            let bits = masks.count_ones();
            out = (out << bits) | part;
        }
        out
    }

    #[inline]
    fn prefix_mask_u32(self, discriminative_bit: u16) -> u32 {
        let disc_byte = bit_byte_index(discriminative_bit);
        let disc_bit_msb0 = bit_in_byte_msb0(discriminative_bit);

        let mut out = 0u32;
        for g in 0..N {
            let pos = self.positions_be[g].to_be_bytes();
            let masks = self.masks_be[g];
            if masks == 0 {
                continue;
            }

            let mut prefix_bytes = [0u8; 8];
            for i in 0..8 {
                let abs_byte = self.base_bytes[g] + u16::from(pos[i]);
                prefix_bytes[i] = if abs_byte < disc_byte {
                    0xFF
                } else if abs_byte == disc_byte {
                    prefix_mask_before_bit_in_byte(disc_bit_msb0)
                } else {
                    0
                };
            }

            let word = u64::from_be_bytes(prefix_bytes);
            let part = pext_u64(word, masks) as u32;
            let bits = masks.count_ones();
            out = (out << bits) | part;
        }
        out
    }
}

#[derive(Clone, Copy, Debug)]
enum DiscriminativeBitsRepresentation {
    Single(SingleMaskPartialKeyMapping),
    Multi1(MultiMaskPartialKeyMapping<1>),
    Multi2(MultiMaskPartialKeyMapping<2>),
    Multi4(MultiMaskPartialKeyMapping<4>),
    Multi8(MultiMaskPartialKeyMapping<8>),
}

impl DiscriminativeBitsRepresentation {
    #[inline]
    fn most_significant_bit(self) -> u16 {
        match self {
            Self::Single(m) => m.most_significant_bit,
            Self::Multi1(m) => m.most_significant_bit,
            Self::Multi2(m) => m.most_significant_bit,
            Self::Multi4(m) => m.most_significant_bit,
            Self::Multi8(m) => m.most_significant_bit,
        }
    }

    #[inline]
    fn num_bits(self) -> u16 {
        match self {
            Self::Single(m) => m.num_bits(),
            Self::Multi1(m) => m.num_bits(),
            Self::Multi2(m) => m.num_bits(),
            Self::Multi4(m) => m.num_bits(),
            Self::Multi8(m) => m.num_bits(),
        }
    }

    #[inline]
    fn extract_u32(self, key: &[u8]) -> u32 {
        match self {
            Self::Single(m) => m.extract_u32(key),
            Self::Multi1(m) => m.extract_u32(key),
            Self::Multi2(m) => m.extract_u32(key),
            Self::Multi4(m) => m.extract_u32(key),
            Self::Multi8(m) => m.extract_u32(key),
        }
    }

    #[inline]
    fn prefix_mask_u32(self, discriminative_bit: u16) -> u32 {
        match self {
            Self::Single(m) => m.prefix_mask_u32(discriminative_bit),
            Self::Multi1(m) => m.prefix_mask_u32(discriminative_bit),
            Self::Multi2(m) => m.prefix_mask_u32(discriminative_bit),
            Self::Multi4(m) => m.prefix_mask_u32(discriminative_bit),
            Self::Multi8(m) => m.prefix_mask_u32(discriminative_bit),
        }
    }

    fn discriminative_bits(self, out: &mut Vec<u16>) {
        match self {
            Self::Single(m) => m.discriminative_bits(out),
            Self::Multi1(m) => m.discriminative_bits(out),
            Self::Multi2(m) => m.discriminative_bits(out),
            Self::Multi4(m) => m.discriminative_bits(out),
            Self::Multi8(m) => m.discriminative_bits(out),
        }
    }

    fn build_minimal(discriminative_bits: &[u16]) -> Self {
        let bits_needed = discriminative_bits.len();
        if let Some(m) = SingleMaskPartialKeyMapping::try_from_bits(discriminative_bits) {
            return Self::Single(m);
        }
        if let Some(m) = MultiMaskPartialKeyMapping::<1>::try_from_bits(discriminative_bits) {
            return Self::Multi1(m);
        }
        // The original HOT node type set does not include a 16-byte MultiMask with 32-bit partial
        // keys; for >16 key bits we directly use the 32-byte (4-mask) representation.
        if bits_needed <= 16 {
            if let Some(m) = MultiMaskPartialKeyMapping::<2>::try_from_bits(discriminative_bits) {
                return Self::Multi2(m);
            }
        }
        if let Some(m) = MultiMaskPartialKeyMapping::<4>::try_from_bits(discriminative_bits) {
            return Self::Multi4(m);
        }
        if let Some(m) = MultiMaskPartialKeyMapping::<8>::try_from_bits(discriminative_bits) {
            return Self::Multi8(m);
        }
        panic!(
            "cannot build discriminative-bits representation: bits={}, bytes may span >255, require too many extraction groups, or require >64 distinct bytes",
            discriminative_bits.len()
        );
    }
}

// =============================================================================
// Node Arena
// =============================================================================
//
// Common node header (all node types):
// [tag:1][n:1][height:1][reserved:1]
//
// `NODE_TWO_ENTRIES` (n=2) layout:
// [header][disc:2][ptr:5][ptr:5]
//
// HOT compound node (2..=32 entries) layout:
// [header][mapping:var][partial_keys:var][ptrs:5*n]
//
// Child pointers are stored as 40-bit (5-byte) tagged offsets.

const NODE_TWO_ENTRIES: u8 = 2;
const NODE_HOT_SINGLE_MASK_U8: u8 = 3;
const NODE_HOT_SINGLE_MASK_U16: u8 = 4;
const NODE_HOT_SINGLE_MASK_U32: u8 = 5;
const NODE_HOT_MULTI_MASK_8B_U8: u8 = 6;
const NODE_HOT_MULTI_MASK_8B_U16: u8 = 7;
const NODE_HOT_MULTI_MASK_8B_U32: u8 = 8;
const NODE_HOT_MULTI_MASK_16B_U16: u8 = 9;
const NODE_HOT_MULTI_MASK_32B_U32: u8 = 10;
const NODE_HOT_MULTI_MASK_64B_U8: u8 = 11;
const NODE_HOT_MULTI_MASK_64B_U16: u8 = 12;
const NODE_HOT_MULTI_MASK_64B_U32: u8 = 13;

const MAX_COMPOUND_ENTRIES: usize = 32;
const NODE_HEADER_SIZE: usize = 4;
const PTR_SIZE: usize = 5;

const MAX_NODE_SIZE: usize = 512;

#[inline]
fn hot_mapping_size(tag: u8) -> usize {
    match tag {
        NODE_HOT_SINGLE_MASK_U8 | NODE_HOT_SINGLE_MASK_U16 | NODE_HOT_SINGLE_MASK_U32 => 16,
        NODE_HOT_MULTI_MASK_8B_U8 | NODE_HOT_MULTI_MASK_8B_U16 | NODE_HOT_MULTI_MASK_8B_U32 => 24,
        NODE_HOT_MULTI_MASK_16B_U16 => 42,
        NODE_HOT_MULTI_MASK_32B_U32 => 78,
        NODE_HOT_MULTI_MASK_64B_U8 | NODE_HOT_MULTI_MASK_64B_U16 | NODE_HOT_MULTI_MASK_64B_U32 => {
            150
        }
        _ => 0,
    }
}

#[inline]
fn hot_partial_key_size(tag: u8) -> usize {
    match tag {
        NODE_HOT_SINGLE_MASK_U8 | NODE_HOT_MULTI_MASK_8B_U8 => 1,
        NODE_HOT_SINGLE_MASK_U16
        | NODE_HOT_MULTI_MASK_8B_U16
        | NODE_HOT_MULTI_MASK_16B_U16
        | NODE_HOT_MULTI_MASK_64B_U16 => 2,
        NODE_HOT_SINGLE_MASK_U32
        | NODE_HOT_MULTI_MASK_8B_U32
        | NODE_HOT_MULTI_MASK_32B_U32
        | NODE_HOT_MULTI_MASK_64B_U32 => 4,
        NODE_HOT_MULTI_MASK_64B_U8 => 1,
        _ => 0,
    }
}

#[inline]
fn hot_is_hot_node(tag: u8) -> bool {
    matches!(
        tag,
        NODE_HOT_SINGLE_MASK_U8
            | NODE_HOT_SINGLE_MASK_U16
            | NODE_HOT_SINGLE_MASK_U32
            | NODE_HOT_MULTI_MASK_8B_U8
            | NODE_HOT_MULTI_MASK_8B_U16
            | NODE_HOT_MULTI_MASK_8B_U32
            | NODE_HOT_MULTI_MASK_16B_U16
            | NODE_HOT_MULTI_MASK_32B_U32
            | NODE_HOT_MULTI_MASK_64B_U8
            | NODE_HOT_MULTI_MASK_64B_U16
            | NODE_HOT_MULTI_MASK_64B_U32
    )
}

#[inline]
fn hot_node_size(tag: u8, n: usize) -> usize {
    debug_assert!(hot_is_hot_node(tag));
    NODE_HEADER_SIZE + hot_mapping_size(tag) + hot_partial_key_size(tag) * n + PTR_SIZE * n
}

/// Node arena for HOT nodes, with simple size-class free lists.
#[derive(Clone)]
struct NodeArena {
    data: Vec<u8>,
    /// Free lists by exact node byte size.
    free: Vec<Vec<u64>>,
}

impl NodeArena {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            free: (0..=MAX_NODE_SIZE).map(|_| Vec::new()).collect(),
        }
    }

    fn capacity(&self) -> usize {
        self.data.capacity() + self.free.iter().map(|v| v.capacity() * 8).sum::<usize>()
    }

    fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
        for v in &mut self.free {
            v.shrink_to_fit();
        }
    }

    #[inline]
    fn read_ptr(&self, at: usize) -> Ptr {
        Ptr((u64::from(self.data[at]))
            | (u64::from(self.data[at + 1]) << 8)
            | (u64::from(self.data[at + 2]) << 16)
            | (u64::from(self.data[at + 3]) << 24)
            | (u64::from(self.data[at + 4]) << 32))
    }

    #[inline]
    fn write_ptr(&mut self, at: usize, ptr: Ptr) {
        debug_assert!(ptr.0 < (1u64 << 40));
        self.data[at] = ptr.0 as u8;
        self.data[at + 1] = (ptr.0 >> 8) as u8;
        self.data[at + 2] = (ptr.0 >> 16) as u8;
        self.data[at + 3] = (ptr.0 >> 24) as u8;
        self.data[at + 4] = (ptr.0 >> 32) as u8;
    }

    #[inline]
    fn tag(&self, off: u64) -> u8 {
        self.data[off as usize]
    }

    #[inline]
    fn n(&self, off: u64) -> usize {
        self.data[off as usize + 1] as usize
    }

    #[inline]
    fn height(&self, off: u64) -> u8 {
        self.data[off as usize + 2]
    }

    #[inline]
    fn set_height(&mut self, off: u64, height: u8) {
        self.data[off as usize + 2] = height;
    }

    fn alloc_two_entries_node(&mut self, disc: u16, height: u8, left: Ptr, right: Ptr) -> u64 {
        const SIZE: usize = NODE_HEADER_SIZE + 2 + 2 * PTR_SIZE;
        debug_assert!(SIZE <= MAX_NODE_SIZE);

        let off = if let Some(off) = self.free[SIZE].pop() {
            off
        } else {
            let off = self.data.len() as u64;
            self.data.resize(self.data.len() + SIZE, 0);
            off
        };

        let o = off as usize;
        self.data[o] = NODE_TWO_ENTRIES;
        self.data[o + 1] = 2;
        self.data[o + 2] = height;
        self.data[o + 3] = 0;
        let disc_bytes = disc.to_le_bytes();
        self.data[o + 4] = disc_bytes[0];
        self.data[o + 5] = disc_bytes[1];
        self.write_ptr(o + 6, left);
        self.write_ptr(o + 6 + PTR_SIZE, right);

        off
    }

    #[inline]
    fn two_entries_disc(&self, off: u64) -> u16 {
        debug_assert_eq!(self.tag(off), NODE_TWO_ENTRIES);
        let o = off as usize;
        u16::from_le_bytes([self.data[o + 4], self.data[o + 5]])
    }

    #[inline]
    fn two_entries_ptr_at(&self, off: u64, idx: usize) -> Ptr {
        debug_assert_eq!(self.tag(off), NODE_TWO_ENTRIES);
        debug_assert!(idx < 2);
        let base = off as usize + 6;
        self.read_ptr(base + idx * PTR_SIZE)
    }

    #[inline]
    fn two_entries_set_ptr_at(&mut self, off: u64, idx: usize, ptr: Ptr) {
        debug_assert_eq!(self.tag(off), NODE_TWO_ENTRIES);
        debug_assert!(idx < 2);
        let base = off as usize + 6;
        self.write_ptr(base + idx * PTR_SIZE, ptr);
    }

    #[inline]
    fn read_u64_le(&self, at: usize) -> u64 {
        u64::from_le_bytes([
            self.data[at],
            self.data[at + 1],
            self.data[at + 2],
            self.data[at + 3],
            self.data[at + 4],
            self.data[at + 5],
            self.data[at + 6],
            self.data[at + 7],
        ])
    }

    #[inline]
    fn write_u64_le(&mut self, at: usize, v: u64) {
        let b = v.to_le_bytes();
        self.data[at..at + 8].copy_from_slice(&b);
    }

    fn alloc_hot_node(
        &mut self,
        tag: u8,
        height: u8,
        mapping: DiscriminativeBitsRepresentation,
        sparse_partial_keys: &[u32],
        child_ptrs: &[Ptr],
    ) -> u64 {
        debug_assert!(hot_is_hot_node(tag));
        let n = child_ptrs.len();
        debug_assert_eq!(sparse_partial_keys.len(), n);
        debug_assert!(n >= 2 && n <= MAX_COMPOUND_ENTRIES);

        let size = hot_node_size(tag, n);
        debug_assert!(size <= MAX_NODE_SIZE);

        let off = if let Some(off) = self.free[size].pop() {
            off
        } else {
            let off = self.data.len() as u64;
            self.data.resize(self.data.len() + size, 0);
            off
        };

        let o = off as usize;
        self.data[o] = tag;
        self.data[o + 1] = n as u8;
        self.data[o + 2] = height;
        self.data[o + 3] = 0;

        // Mapping
        let map_off = o + NODE_HEADER_SIZE;
        match (tag, mapping) {
            (
                NODE_HOT_SINGLE_MASK_U8 | NODE_HOT_SINGLE_MASK_U16 | NODE_HOT_SINGLE_MASK_U32,
                DiscriminativeBitsRepresentation::Single(m),
            ) => {
                self.data[map_off..map_off + 2]
                    .copy_from_slice(&m.most_significant_bit.to_le_bytes());
                self.data[map_off + 2..map_off + 4]
                    .copy_from_slice(&m.least_significant_bit.to_le_bytes());
                self.data[map_off + 4..map_off + 6].copy_from_slice(&m.byte_offset.to_le_bytes());
                self.data[map_off + 6..map_off + 8].copy_from_slice(&0u16.to_le_bytes());
                self.write_u64_le(map_off + 8, m.extraction_mask);
            }
            (
                NODE_HOT_MULTI_MASK_8B_U8 | NODE_HOT_MULTI_MASK_8B_U16 | NODE_HOT_MULTI_MASK_8B_U32,
                DiscriminativeBitsRepresentation::Multi1(m),
            ) => {
                self.data[map_off..map_off + 2]
                    .copy_from_slice(&m.most_significant_bit.to_le_bytes());
                self.data[map_off + 2..map_off + 4]
                    .copy_from_slice(&m.least_significant_bit.to_le_bytes());
                self.data[map_off + 4..map_off + 6].copy_from_slice(&m.base_bytes[0].to_le_bytes());
                self.data[map_off + 6] = m.used_bytes;
                self.data[map_off + 7] = 0;
                self.write_u64_le(map_off + 8, m.positions_be[0]);
                self.write_u64_le(map_off + 16, m.masks_be[0]);
            }
            (NODE_HOT_MULTI_MASK_16B_U16, DiscriminativeBitsRepresentation::Multi2(m)) => {
                self.data[map_off..map_off + 2]
                    .copy_from_slice(&m.most_significant_bit.to_le_bytes());
                self.data[map_off + 2..map_off + 4]
                    .copy_from_slice(&m.least_significant_bit.to_le_bytes());
                self.data[map_off + 4..map_off + 6].copy_from_slice(&m.base_bytes[0].to_le_bytes());
                self.data[map_off + 6..map_off + 8].copy_from_slice(&m.base_bytes[1].to_le_bytes());
                self.data[map_off + 8] = m.used_bytes;
                self.data[map_off + 9] = 0;
                self.write_u64_le(map_off + 10, m.positions_be[0]);
                self.write_u64_le(map_off + 18, m.positions_be[1]);
                self.write_u64_le(map_off + 26, m.masks_be[0]);
                self.write_u64_le(map_off + 34, m.masks_be[1]);
            }
            (NODE_HOT_MULTI_MASK_32B_U32, DiscriminativeBitsRepresentation::Multi4(m)) => {
                self.data[map_off..map_off + 2]
                    .copy_from_slice(&m.most_significant_bit.to_le_bytes());
                self.data[map_off + 2..map_off + 4]
                    .copy_from_slice(&m.least_significant_bit.to_le_bytes());
                self.data[map_off + 4..map_off + 6].copy_from_slice(&m.base_bytes[0].to_le_bytes());
                self.data[map_off + 6..map_off + 8].copy_from_slice(&m.base_bytes[1].to_le_bytes());
                self.data[map_off + 8..map_off + 10]
                    .copy_from_slice(&m.base_bytes[2].to_le_bytes());
                self.data[map_off + 10..map_off + 12]
                    .copy_from_slice(&m.base_bytes[3].to_le_bytes());
                self.data[map_off + 12] = m.used_bytes;
                self.data[map_off + 13] = 0;
                self.write_u64_le(map_off + 14, m.positions_be[0]);
                self.write_u64_le(map_off + 22, m.positions_be[1]);
                self.write_u64_le(map_off + 30, m.positions_be[2]);
                self.write_u64_le(map_off + 38, m.positions_be[3]);
                self.write_u64_le(map_off + 46, m.masks_be[0]);
                self.write_u64_le(map_off + 54, m.masks_be[1]);
                self.write_u64_le(map_off + 62, m.masks_be[2]);
                self.write_u64_le(map_off + 70, m.masks_be[3]);
            }
            (
                NODE_HOT_MULTI_MASK_64B_U8
                | NODE_HOT_MULTI_MASK_64B_U16
                | NODE_HOT_MULTI_MASK_64B_U32,
                DiscriminativeBitsRepresentation::Multi8(m),
            ) => {
                self.data[map_off..map_off + 2]
                    .copy_from_slice(&m.most_significant_bit.to_le_bytes());
                self.data[map_off + 2..map_off + 4]
                    .copy_from_slice(&m.least_significant_bit.to_le_bytes());
                for g in 0..8 {
                    let at = map_off + 4 + g * 2;
                    self.data[at..at + 2].copy_from_slice(&m.base_bytes[g].to_le_bytes());
                }
                self.data[map_off + 20] = m.used_bytes;
                self.data[map_off + 21] = 0;

                for g in 0..8 {
                    self.write_u64_le(map_off + 22 + g * 8, m.positions_be[g]);
                }
                for g in 0..8 {
                    self.write_u64_le(map_off + 86 + g * 8, m.masks_be[g]);
                }
            }
            _ => {
                panic!(
                    "alloc_hot_node: mapping/tag mismatch: tag={tag} mapping={:?}",
                    mapping
                );
            }
        }

        // Sparse partial keys
        let pk_size = hot_partial_key_size(tag);
        let pk_off = map_off + hot_mapping_size(tag);
        for i in 0..n {
            let pk = sparse_partial_keys[i];
            let at = pk_off + i * pk_size;
            match pk_size {
                1 => {
                    debug_assert!(pk <= u32::from(u8::MAX));
                    self.data[at] = pk as u8;
                }
                2 => {
                    debug_assert!(pk <= u32::from(u16::MAX));
                    self.data[at..at + 2].copy_from_slice(&(pk as u16).to_le_bytes());
                }
                4 => {
                    self.data[at..at + 4].copy_from_slice(&pk.to_le_bytes());
                }
                _ => unreachable!(),
            }
        }

        // Child pointers
        let ptr_off = pk_off + pk_size * n;
        for i in 0..n {
            self.write_ptr(ptr_off + i * PTR_SIZE, child_ptrs[i]);
        }

        off
    }

    #[inline]
    fn hot_mapping(&self, off: u64) -> DiscriminativeBitsRepresentation {
        let tag = self.tag(off);
        debug_assert!(hot_is_hot_node(tag));
        let o = off as usize;
        let map_off = o + NODE_HEADER_SIZE;
        match tag {
            NODE_HOT_SINGLE_MASK_U8 | NODE_HOT_SINGLE_MASK_U16 | NODE_HOT_SINGLE_MASK_U32 => {
                let msb = u16::from_le_bytes([self.data[map_off], self.data[map_off + 1]]);
                let lsb = u16::from_le_bytes([self.data[map_off + 2], self.data[map_off + 3]]);
                let byte_offset =
                    u16::from_le_bytes([self.data[map_off + 4], self.data[map_off + 5]]);
                let extraction_mask = self.read_u64_le(map_off + 8);
                DiscriminativeBitsRepresentation::Single(SingleMaskPartialKeyMapping {
                    most_significant_bit: msb,
                    least_significant_bit: lsb,
                    byte_offset,
                    extraction_mask,
                })
            }
            NODE_HOT_MULTI_MASK_8B_U8 | NODE_HOT_MULTI_MASK_8B_U16 | NODE_HOT_MULTI_MASK_8B_U32 => {
                let msb = u16::from_le_bytes([self.data[map_off], self.data[map_off + 1]]);
                let lsb = u16::from_le_bytes([self.data[map_off + 2], self.data[map_off + 3]]);
                let base_byte =
                    u16::from_le_bytes([self.data[map_off + 4], self.data[map_off + 5]]);
                let used_bytes = self.data[map_off + 6];
                let positions_be = [self.read_u64_le(map_off + 8)];
                let masks_be = [self.read_u64_le(map_off + 16)];
                DiscriminativeBitsRepresentation::Multi1(MultiMaskPartialKeyMapping {
                    most_significant_bit: msb,
                    least_significant_bit: lsb,
                    base_bytes: [base_byte],
                    used_bytes,
                    positions_be,
                    masks_be,
                })
            }
            NODE_HOT_MULTI_MASK_16B_U16 => {
                let msb = u16::from_le_bytes([self.data[map_off], self.data[map_off + 1]]);
                let lsb = u16::from_le_bytes([self.data[map_off + 2], self.data[map_off + 3]]);
                let base0 = u16::from_le_bytes([self.data[map_off + 4], self.data[map_off + 5]]);
                let base1 = u16::from_le_bytes([self.data[map_off + 6], self.data[map_off + 7]]);
                let used_bytes = self.data[map_off + 8];
                let positions_be = [
                    self.read_u64_le(map_off + 10),
                    self.read_u64_le(map_off + 18),
                ];
                let masks_be = [
                    self.read_u64_le(map_off + 26),
                    self.read_u64_le(map_off + 34),
                ];
                DiscriminativeBitsRepresentation::Multi2(MultiMaskPartialKeyMapping {
                    most_significant_bit: msb,
                    least_significant_bit: lsb,
                    base_bytes: [base0, base1],
                    used_bytes,
                    positions_be,
                    masks_be,
                })
            }
            NODE_HOT_MULTI_MASK_32B_U32 => {
                let msb = u16::from_le_bytes([self.data[map_off], self.data[map_off + 1]]);
                let lsb = u16::from_le_bytes([self.data[map_off + 2], self.data[map_off + 3]]);
                let base0 = u16::from_le_bytes([self.data[map_off + 4], self.data[map_off + 5]]);
                let base1 = u16::from_le_bytes([self.data[map_off + 6], self.data[map_off + 7]]);
                let base2 = u16::from_le_bytes([self.data[map_off + 8], self.data[map_off + 9]]);
                let base3 = u16::from_le_bytes([self.data[map_off + 10], self.data[map_off + 11]]);
                let used_bytes = self.data[map_off + 12];
                let positions_be = [
                    self.read_u64_le(map_off + 14),
                    self.read_u64_le(map_off + 22),
                    self.read_u64_le(map_off + 30),
                    self.read_u64_le(map_off + 38),
                ];
                let masks_be = [
                    self.read_u64_le(map_off + 46),
                    self.read_u64_le(map_off + 54),
                    self.read_u64_le(map_off + 62),
                    self.read_u64_le(map_off + 70),
                ];
                DiscriminativeBitsRepresentation::Multi4(MultiMaskPartialKeyMapping {
                    most_significant_bit: msb,
                    least_significant_bit: lsb,
                    base_bytes: [base0, base1, base2, base3],
                    used_bytes,
                    positions_be,
                    masks_be,
                })
            }
            NODE_HOT_MULTI_MASK_64B_U8
            | NODE_HOT_MULTI_MASK_64B_U16
            | NODE_HOT_MULTI_MASK_64B_U32 => {
                let msb = u16::from_le_bytes([self.data[map_off], self.data[map_off + 1]]);
                let lsb = u16::from_le_bytes([self.data[map_off + 2], self.data[map_off + 3]]);

                let mut base_bytes = [0u16; 8];
                for g in 0..8 {
                    let at = map_off + 4 + g * 2;
                    base_bytes[g] = u16::from_le_bytes([self.data[at], self.data[at + 1]]);
                }
                let used_bytes = self.data[map_off + 20];

                let mut positions_be = [0u64; 8];
                let mut masks_be = [0u64; 8];
                for g in 0..8 {
                    positions_be[g] = self.read_u64_le(map_off + 22 + g * 8);
                }
                for g in 0..8 {
                    masks_be[g] = self.read_u64_le(map_off + 86 + g * 8);
                }

                DiscriminativeBitsRepresentation::Multi8(MultiMaskPartialKeyMapping {
                    most_significant_bit: msb,
                    least_significant_bit: lsb,
                    base_bytes,
                    used_bytes,
                    positions_be,
                    masks_be,
                })
            }
            _ => unreachable!("hot_mapping called for non-hot tag {tag}"),
        }
    }

    #[inline]
    fn hot_partial_key_u32_at(&self, off: u64, idx: usize) -> u32 {
        let tag = self.tag(off);
        debug_assert!(hot_is_hot_node(tag));
        let n = self.n(off);
        debug_assert!(idx < n);
        let pk_size = hot_partial_key_size(tag);
        let base = off as usize + NODE_HEADER_SIZE + hot_mapping_size(tag);
        let at = base + idx * pk_size;
        match pk_size {
            1 => u32::from(self.data[at]),
            2 => u32::from(u16::from_le_bytes([self.data[at], self.data[at + 1]])),
            4 => u32::from_le_bytes([
                self.data[at],
                self.data[at + 1],
                self.data[at + 2],
                self.data[at + 3],
            ]),
            _ => unreachable!(),
        }
    }

    #[inline]
    fn hot_entry_ptr_at(&self, off: u64, idx: usize) -> Ptr {
        let tag = self.tag(off);
        debug_assert!(hot_is_hot_node(tag));
        let n = self.n(off);
        debug_assert!(idx < n);
        let pk_size = hot_partial_key_size(tag);
        let pk_base = off as usize + NODE_HEADER_SIZE + hot_mapping_size(tag);
        let ptr_base = pk_base + pk_size * n;
        self.read_ptr(ptr_base + idx * PTR_SIZE)
    }

    #[inline]
    fn hot_set_entry_ptr_at(&mut self, off: u64, idx: usize, ptr: Ptr) {
        let tag = self.tag(off);
        debug_assert!(hot_is_hot_node(tag));
        let n = self.n(off);
        debug_assert!(idx < n);
        let pk_size = hot_partial_key_size(tag);
        let pk_base = off as usize + NODE_HEADER_SIZE + hot_mapping_size(tag);
        let ptr_base = pk_base + pk_size * n;
        self.write_ptr(ptr_base + idx * PTR_SIZE, ptr);
    }

    #[inline]
    fn node_size(&self, off: u64) -> usize {
        let tag = self.tag(off);
        let n = self.n(off);
        match tag {
            NODE_TWO_ENTRIES => NODE_HEADER_SIZE + 2 + 2 * PTR_SIZE,
            t if hot_is_hot_node(t) => hot_node_size(t, n),
            _ => panic!("unknown node tag {tag}"),
        }
    }

    #[inline]
    fn free_node(&mut self, off: u64) {
        let size = self.node_size(off);
        debug_assert!(size <= MAX_NODE_SIZE);
        self.free[size].push(off);
    }
}

// =============================================================================
// HotTree with adaptive prefix compression
// =============================================================================

/// A memory-efficient ordered map using Height Optimized Trie.
///
/// Features:
/// - Arena-based leaf storage with prefix compression
/// - HOT compound nodes (2..=32 entries)
/// - Adaptive prefix learning from natural delimiters
/// - ZST value optimization
pub struct HotTree<V> {
    // === Prefix compression ===
    /// Prefix pool: contiguous storage of all prefixes
    prefix_pool: Vec<u8>,
    /// Offset of each prefix in pool (prefix_id -> offset)
    prefix_offsets: Vec<u32>,
    /// Map from prefix hash to prefix_id for fast lookup
    prefix_hash: HashMap<u64, u16>,

    // === Leaf storage ===
    /// Leaf arena: [prefix_id:2][suffix_len:1-3][suffix...][value_idx:4]
    leaves: Vec<u8>,

    // === Values ===
    values: Vec<Option<V>>,
    /// ZST values: stored only to preserve `Drop` semantics while using no heap bytes.
    zst_values: Vec<V>,

    // === Trie structure ===
    /// HOT node arena
    nodes: NodeArena,
    root: Ptr,
    count: usize,

    _marker: PhantomData<V>,
}

impl<V> HotTree<V> {
    pub fn new() -> Self {
        let mut tree = Self {
            prefix_pool: Vec::new(),
            prefix_offsets: Vec::new(),
            prefix_hash: HashMap::new(),
            leaves: Vec::new(),
            values: Vec::new(),
            zst_values: Vec::new(),
            nodes: NodeArena::new(),
            root: Ptr::NULL,
            count: 0,
            _marker: PhantomData,
        };
        // Register empty prefix as ID 0
        tree.register_prefix(&[]);
        tree
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn memory_usage(&self) -> usize {
        self.prefix_pool.capacity()
            + self.prefix_offsets.capacity() * 4
            + self.prefix_hash.capacity() * 16
            + self.leaves.capacity()
            + self.values.capacity() * std::mem::size_of::<Option<V>>()
            + self.zst_values.capacity() * std::mem::size_of::<V>()
            + self.nodes.capacity()
    }

    pub fn shrink_to_fit(&mut self) {
        self.prefix_pool.shrink_to_fit();
        self.prefix_offsets.shrink_to_fit();
        self.prefix_hash.shrink_to_fit();
        self.leaves.shrink_to_fit();
        self.values.shrink_to_fit();
        self.zst_values.shrink_to_fit();
        self.nodes.shrink_to_fit();
    }

    /// Compact the node arena by rebuilding live nodes into a fresh arena.
    ///
    /// This can reduce memory when the arena has accumulated holes due to node
    /// replacements during insertion. Returns the number of nodes rewritten.
    pub fn compact(&mut self) -> usize {
        if self.root.is_null() || self.root.is_leaf() {
            return 0;
        }

        let old_nodes = std::mem::replace(&mut self.nodes, NodeArena::new());
        let mut new_nodes = NodeArena::new();
        let mut rewritten = 0usize;

        #[derive(Clone, Copy)]
        struct Frame {
            old_off: u64,
            tag: u8,
            n: usize,
            height: u8,
            next_child: usize,
            parent_slot: Option<usize>,
            ptrs: [Ptr; MAX_COMPOUND_ENTRIES],
        }

        fn load_frame(old_nodes: &NodeArena, ptr: Ptr, parent_slot: Option<usize>) -> Frame {
            let off = ptr.node_off();
            let tag = old_nodes.tag(off);
            let n = old_nodes.n(off);
            let height = old_nodes.height(off);

            let mut ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
            match tag {
                NODE_TWO_ENTRIES => {
                    debug_assert_eq!(n, 2);
                    ptrs[0] = old_nodes.two_entries_ptr_at(off, 0);
                    ptrs[1] = old_nodes.two_entries_ptr_at(off, 1);
                }
                t if hot_is_hot_node(t) => {
                    for i in 0..n {
                        ptrs[i] = old_nodes.hot_entry_ptr_at(off, i);
                    }
                }
                other => panic!("unknown node tag {other}"),
            }

            Frame {
                old_off: off,
                tag,
                n,
                height,
                next_child: 0,
                parent_slot,
                ptrs,
            }
        }

        let mut stack: Vec<Frame> = Vec::with_capacity(64);
        stack.push(load_frame(&old_nodes, self.root, None));

        let mut new_root = Ptr::NULL;
        while let Some(frame) = stack.last_mut() {
            if frame.next_child < frame.n {
                let idx = frame.next_child;
                frame.next_child += 1;
                let child = frame.ptrs[idx];
                if !child.is_leaf() {
                    stack.push(load_frame(&old_nodes, child, Some(idx)));
                }
                continue;
            }

            let frame = stack.pop().expect("non-empty stack");
            let new_ptr = match frame.tag {
                NODE_TWO_ENTRIES => {
                    debug_assert_eq!(frame.n, 2);
                    let disc = old_nodes.two_entries_disc(frame.old_off);
                    let new_off = new_nodes.alloc_two_entries_node(
                        disc,
                        frame.height,
                        frame.ptrs[0],
                        frame.ptrs[1],
                    );
                    Ptr::node(new_off)
                }
                t if hot_is_hot_node(t) => {
                    let mapping = old_nodes.hot_mapping(frame.old_off);
                    let tag = Self::hot_tag_for(mapping);
                    let mut sparse = [0u32; MAX_COMPOUND_ENTRIES];
                    for i in 0..frame.n {
                        sparse[i] = old_nodes.hot_partial_key_u32_at(frame.old_off, i);
                    }
                    let new_off = new_nodes.alloc_hot_node(
                        tag,
                        frame.height,
                        mapping,
                        &sparse[..frame.n],
                        &frame.ptrs[..frame.n],
                    );
                    Ptr::node(new_off)
                }
                other => panic!("unknown node tag {other}"),
            };

            rewritten += 1;
            if let Some(parent_slot) = frame.parent_slot {
                let parent = stack.last_mut().expect("parent frame must exist");
                parent.ptrs[parent_slot] = new_ptr;
            } else {
                new_root = new_ptr;
            }
        }

        self.nodes = new_nodes;
        self.root = new_root;
        rewritten
    }

    /// FNV-1a hash for prefix lookup
    #[inline]
    fn hash_prefix(prefix: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for &byte in prefix {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    // =========================================================================
    // Prefix compression
    // =========================================================================

    /// Extract natural prefix from key (up to delimiter)
    fn extract_natural_prefix(key: &[u8]) -> &[u8] {
        if key.len() < MIN_PREFIX_LEN {
            return &[];
        }

        // Look for delimiter within acceptable range
        let max_check = key.len().min(MAX_PREFIX_LEN);
        for i in MIN_PREFIX_LEN..max_check {
            let b = key[i];
            // Natural delimiters: / : \ (common in URLs, paths, URIs)
            if b == b'/' || b == b':' || b == b'\\' {
                return &key[..=i]; // Include delimiter
            }
        }
        &[]
    }

    /// Register a prefix, returns its ID
    fn register_prefix(&mut self, prefix: &[u8]) -> u16 {
        let hash = Self::hash_prefix(prefix);

        // Check if already exists
        if let Some(&id) = self.prefix_hash.get(&hash) {
            // Verify it's actually the same prefix (handle hash collisions)
            let stored = self.get_prefix(id);
            if stored == prefix {
                return id;
            }
            // Hash collision - fall back to empty prefix to be safe
            return 0;
        }

        if self.prefix_offsets.len() >= MAX_PREFIXES {
            return 0; // Fall back to empty prefix
        }

        let id = self.prefix_offsets.len() as u16;

        // Store offset and prefix bytes
        let offset = self.prefix_pool.len() as u32;
        self.prefix_offsets.push(offset);
        self.prefix_pool.extend_from_slice(prefix);

        self.prefix_hash.insert(hash, id);
        id
    }

    /// Get or create prefix for a key
    fn get_or_create_prefix(&mut self, key: &[u8]) -> (u16, usize) {
        let natural = Self::extract_natural_prefix(key);
        if natural.is_empty() {
            return (0, 0); // Empty prefix
        }

        let hash = Self::hash_prefix(natural);

        // Check if prefix exists
        if let Some(&id) = self.prefix_hash.get(&hash) {
            let stored = self.get_prefix(id);
            if stored == natural {
                return (id, natural.len());
            }
            // Hash collision - use empty prefix
            return (0, 0);
        }

        // Register new prefix
        let id = self.register_prefix(natural);
        (id, if id == 0 { 0 } else { natural.len() })
    }

    /// Get prefix bytes for a prefix ID (O(1) lookup)
    #[inline]
    fn get_prefix(&self, id: u16) -> &[u8] {
        let idx = id as usize;
        if idx >= self.prefix_offsets.len() {
            return &[];
        }

        let start = self.prefix_offsets[idx] as usize;
        let end = if idx + 1 < self.prefix_offsets.len() {
            self.prefix_offsets[idx + 1] as usize
        } else {
            self.prefix_pool.len()
        };

        &self.prefix_pool[start..end]
    }

    // =========================================================================
    // Leaf operations
    // =========================================================================

    /// Store leaf with prefix compression
    /// Format: [prefix_id:2][suffix_len:1-3][suffix...][value_idx:4]
    ///
    /// suffix_len encoding:
    /// - If < 255: [len:1]
    /// - If >= 255: [0xFF][len:2]
    ///
    /// Returns a leaf pointer (byte offset into `leaves`).
    fn store_leaf(&mut self, key: &[u8]) -> Ptr {
        let (prefix_id, prefix_len) = self.get_or_create_prefix(key);
        let suffix = &key[prefix_len..];

        // Record byte offset and create leaf pointer (38-bit offset)
        let byte_offset = self.leaves.len() as u64;
        if byte_offset > Ptr::OFFSET_MASK {
            panic!(
                "LEAF ARENA OVERFLOW: leaves.len()={} exceeds max offset {}",
                byte_offset,
                Ptr::OFFSET_MASK
            );
        }
        let leaf_ptr = Ptr::leaf(byte_offset);

        // Store prefix_id (2 bytes)
        self.leaves.extend_from_slice(&prefix_id.to_le_bytes());

        // Store suffix_len (variable length - 1 byte for < 255, 3 bytes for >= 255)
        let suffix_len = suffix.len();
        if suffix_len < 255 {
            self.leaves.push(suffix_len as u8);
        } else {
            self.leaves.push(0xFF);
            self.leaves
                .extend_from_slice(&(suffix_len as u16).to_le_bytes());
        }

        // Store suffix
        self.leaves.extend_from_slice(suffix);

        // Store value_idx (4 bytes) if not ZST
        if std::mem::size_of::<V>() > 0 {
            let value_idx = self.values.len() as u32;
            self.leaves.extend_from_slice(&value_idx.to_le_bytes());
        }

        leaf_ptr
    }

    /// Read suffix_len and return (suffix_len, bytes_consumed_for_header)
    #[inline]
    fn read_suffix_len(&self, off: usize) -> (usize, usize) {
        let first = self.leaves[off];
        if first < 255 {
            (first as usize, 1)
        } else {
            let len = u16::from_le_bytes([self.leaves[off + 1], self.leaves[off + 2]]);
            (len as usize, 3)
        }
    }

    /// Reconstruct full key from a leaf offset.
    fn get_leaf_key(&self, leaf_off: u64) -> Vec<u8> {
        let o = leaf_off as usize;
        let prefix_id = u16::from_le_bytes([self.leaves[o], self.leaves[o + 1]]);
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let suffix_start = o + 2 + slen_bytes;
        let suffix = &self.leaves[suffix_start..suffix_start + suffix_len];

        let prefix = self.get_prefix(prefix_id);
        let mut key = Vec::with_capacity(prefix.len() + suffix.len());
        key.extend_from_slice(prefix);
        key.extend_from_slice(suffix);
        key
    }

    fn get_leaf_value_idx(&self, leaf_off: u64) -> usize {
        debug_assert_ne!(std::mem::size_of::<V>(), 0);
        let o = leaf_off as usize;
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let val_off = o + 2 + slen_bytes + suffix_len;
        u32::from_le_bytes([
            self.leaves[val_off],
            self.leaves[val_off + 1],
            self.leaves[val_off + 2],
            self.leaves[val_off + 3],
        ]) as usize
    }

    #[inline]
    fn leaf_key_equals(&self, leaf_off: u64, key: &[u8]) -> bool {
        let o = leaf_off as usize;
        let prefix_id = u16::from_le_bytes([self.leaves[o], self.leaves[o + 1]]);
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let suffix_start = o + 2 + slen_bytes;

        let prefix = self.get_prefix(prefix_id);
        if key.len() != prefix.len() + suffix_len {
            return false;
        }
        if !key.starts_with(prefix) {
            return false;
        }
        let key_suffix = &key[prefix.len()..];
        let leaf_suffix = &self.leaves[suffix_start..suffix_start + suffix_len];
        key_suffix == leaf_suffix
    }

    fn first_diff_bit_leaf(&self, leaf_off: u64, other: &[u8]) -> Option<u16> {
        let o = leaf_off as usize;
        let prefix_id = u16::from_le_bytes([self.leaves[o], self.leaves[o + 1]]);
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let suffix_start = o + 2 + slen_bytes;

        let prefix = self.get_prefix(prefix_id);
        let suffix = &self.leaves[suffix_start..suffix_start + suffix_len];
        let leaf_len = prefix.len() + suffix.len();
        let max_len = leaf_len.max(other.len());

        for i in 0..max_len {
            let ab = if i < prefix.len() {
                prefix[i]
            } else if i < leaf_len {
                suffix[i - prefix.len()]
            } else {
                0
            };
            let bb = other.get(i).copied().unwrap_or(0);
            if ab != bb {
                let xor = ab ^ bb;
                return Some(i as u16 * 8 + xor.leading_zeros() as u16);
            }
        }
        None
    }

    #[inline]
    fn zst_value_ref(&self) -> &V {
        debug_assert_eq!(std::mem::size_of::<V>(), 0);
        // SAFETY: for ZSTs, any non-null properly aligned pointer is a valid reference.
        unsafe { &*std::ptr::NonNull::<V>::dangling().as_ptr() }
    }

    #[inline]
    fn bit_at_leaf(&self, leaf_off: u64, pos: u16) -> u8 {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);

        let o = leaf_off as usize;
        let prefix_id = u16::from_le_bytes([self.leaves[o], self.leaves[o + 1]]);
        let (suffix_len, slen_bytes) = self.read_suffix_len(o + 2);
        let suffix_start = o + 2 + slen_bytes;

        let prefix = self.get_prefix(prefix_id);
        let leaf_len = prefix.len() + suffix_len;
        let byte = if byte_idx < prefix.len() {
            prefix[byte_idx]
        } else if byte_idx < leaf_len {
            self.leaves[suffix_start + (byte_idx - prefix.len())]
        } else {
            0
        };

        (byte >> bit_idx) & 1
    }

    // =========================================================================
    // Generic node operations (TwoEntries / HOT)
    // =========================================================================

    #[inline]
    fn node_entry_count(&self, node_off: u64) -> usize {
        self.nodes.n(node_off)
    }

    #[inline]
    fn node_most_significant_bit(&self, node_off: u64) -> u16 {
        match self.nodes.tag(node_off) {
            NODE_TWO_ENTRIES => self.nodes.two_entries_disc(node_off),
            tag if hot_is_hot_node(tag) => self.nodes.hot_mapping(node_off).most_significant_bit(),
            other => panic!("unknown node tag {other}"),
        }
    }

    #[inline]
    fn node_entry_ptr(&self, node_off: u64, entry_idx: usize) -> Ptr {
        match self.nodes.tag(node_off) {
            NODE_TWO_ENTRIES => self.nodes.two_entries_ptr_at(node_off, entry_idx),
            tag if hot_is_hot_node(tag) => self.nodes.hot_entry_ptr_at(node_off, entry_idx),
            other => panic!("unknown node tag {other}"),
        }
    }

    #[inline]
    fn node_set_entry_ptr(&mut self, node_off: u64, entry_idx: usize, ptr: Ptr) {
        match self.nodes.tag(node_off) {
            NODE_TWO_ENTRIES => self.nodes.two_entries_set_ptr_at(node_off, entry_idx, ptr),
            tag if hot_is_hot_node(tag) => {
                self.nodes.hot_set_entry_ptr_at(node_off, entry_idx, ptr)
            }
            other => panic!("unknown node tag {other}"),
        }
    }

    #[inline]
    fn hot_search_mask(&self, node_off: u64, dense_key: u32) -> u32 {
        let tag = self.nodes.tag(node_off);
        let n = self.node_entry_count(node_off);
        debug_assert!(n >= 2 && n <= 32);

        #[cfg(target_arch = "x86_64")]
        {
            if std::is_x86_feature_detected!("avx2") {
                let pk_size = hot_partial_key_size(tag);
                let pk_base = node_off as usize + NODE_HEADER_SIZE + hot_mapping_size(tag);
                let pk_bytes = &self.nodes.data[pk_base..pk_base + pk_size * n];

                let n_mask = if n == 32 { !0u32 } else { (1u32 << n) - 1 };
                let mask = match pk_size {
                    1 => {
                        let mut buf = [0u8; 32];
                        buf[..n].copy_from_slice(pk_bytes);
                        // SAFETY: guarded by `is_x86_feature_detected!("avx2")`.
                        unsafe { Self::hot_search_mask_avx2_u8(&buf, dense_key as u8) }
                    }
                    2 => {
                        let mut buf = [0u16; 32];
                        // SAFETY: `buf` is a plain old data array; we copy exactly `2*n` bytes.
                        let buf_bytes = unsafe {
                            std::slice::from_raw_parts_mut(
                                buf.as_mut_ptr() as *mut u8,
                                std::mem::size_of_val(&buf),
                            )
                        };
                        buf_bytes[..pk_bytes.len()].copy_from_slice(pk_bytes);
                        // SAFETY: guarded by `is_x86_feature_detected!("avx2")`.
                        unsafe { Self::hot_search_mask_avx2_u16(&buf, dense_key as u16) }
                    }
                    4 => {
                        let mut buf = [0u32; 32];
                        // SAFETY: `buf` is a plain old data array; we copy exactly `4*n` bytes.
                        let buf_bytes = unsafe {
                            std::slice::from_raw_parts_mut(
                                buf.as_mut_ptr() as *mut u8,
                                std::mem::size_of_val(&buf),
                            )
                        };
                        buf_bytes[..pk_bytes.len()].copy_from_slice(pk_bytes);
                        // SAFETY: guarded by `is_x86_feature_detected!("avx2")`.
                        unsafe { Self::hot_search_mask_avx2_u32(&buf, dense_key) }
                    }
                    _ => 0,
                };
                return mask & n_mask;
            }
        }

        let mut mask = 0u32;
        for i in 0..n {
            let pk = self.nodes.hot_partial_key_u32_at(node_off, i);
            if (dense_key & pk) == pk {
                mask |= 1u32 << i;
            }
        }
        mask
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn hot_search_mask_avx2_u8(partials: &[u8; 32], dense: u8) -> u32 {
        use std::arch::x86_64::*;
        unsafe {
            let search = _mm256_set1_epi8(dense as i8);
            let haystack = _mm256_loadu_si256(partials.as_ptr() as *const __m256i);
            let search_result = _mm256_cmpeq_epi8(_mm256_and_si256(haystack, search), haystack);
            _mm256_movemask_epi8(search_result) as u32
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn hot_search_mask_avx2_u16(partials: &[u16; 32], dense: u16) -> u32 {
        use std::arch::x86_64::*;
        unsafe {
            let search = _mm256_set1_epi16(dense as i16);

            let haystack1 = _mm256_loadu_si256(partials.as_ptr() as *const __m256i);
            let haystack2 = _mm256_loadu_si256(partials.as_ptr().add(16) as *const __m256i);

            let perm_mask = _mm256_set_epi32(7, 6, 3, 2, 5, 4, 1, 0);
            let search_result1 = _mm256_cmpeq_epi16(_mm256_and_si256(haystack1, search), haystack1);
            let search_result2 = _mm256_cmpeq_epi16(_mm256_and_si256(haystack2, search), haystack2);

            let intermediate = _mm256_permutevar8x32_epi32(
                _mm256_packs_epi16(search_result1, search_result2),
                perm_mask,
            );
            _mm256_movemask_epi8(intermediate) as u32
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn hot_search_mask_avx2_u32(partials: &[u32; 32], dense: u32) -> u32 {
        use std::arch::x86_64::*;
        unsafe {
            let search = _mm256_set1_epi32(dense as i32);

            let haystack1 = _mm256_loadu_si256(partials.as_ptr() as *const __m256i);
            let haystack2 = _mm256_loadu_si256(partials.as_ptr().add(8) as *const __m256i);
            let haystack3 = _mm256_loadu_si256(partials.as_ptr().add(16) as *const __m256i);
            let haystack4 = _mm256_loadu_si256(partials.as_ptr().add(24) as *const __m256i);

            let perm_mask = _mm256_set_epi32(7, 6, 3, 2, 5, 4, 1, 0);

            let search_result1 = _mm256_cmpeq_epi32(_mm256_and_si256(haystack1, search), haystack1);
            let search_result2 = _mm256_cmpeq_epi32(_mm256_and_si256(haystack2, search), haystack2);
            let search_result3 = _mm256_cmpeq_epi32(_mm256_and_si256(haystack3, search), haystack3);
            let search_result4 = _mm256_cmpeq_epi32(_mm256_and_si256(haystack4, search), haystack4);

            let packed1 = _mm256_permutevar8x32_epi32(
                _mm256_packs_epi32(search_result1, search_result2),
                perm_mask,
            );
            let packed2 = _mm256_permutevar8x32_epi32(
                _mm256_packs_epi32(search_result3, search_result4),
                perm_mask,
            );
            let intermediate =
                _mm256_permutevar8x32_epi32(_mm256_packs_epi16(packed1, packed2), perm_mask);
            _mm256_movemask_epi8(intermediate) as u32
        }
    }

    #[inline]
    fn node_descend_index(&self, node_off: u64, key: &[u8]) -> usize {
        match self.nodes.tag(node_off) {
            NODE_TWO_ENTRIES => {
                let disc = self.nodes.two_entries_disc(node_off);
                usize::from(Self::bit_at(key, disc))
            }
            tag if hot_is_hot_node(tag) => {
                let mapping = self.nodes.hot_mapping(node_off);
                let dense = mapping.extract_u32(key);
                let mask = self.hot_search_mask(node_off, dense);
                debug_assert_ne!(mask, 0);
                let idx = 31u32.saturating_sub(mask.leading_zeros());
                idx as usize
            }
            other => panic!("unknown node tag {other}"),
        }
    }

    #[inline]
    fn node_descend(&self, node_off: u64, key: &[u8]) -> Ptr {
        let entry_idx = self.node_descend_index(node_off, key);
        self.node_entry_ptr(node_off, entry_idx)
    }

    // =========================================================================
    // Bit operations
    // =========================================================================

    #[inline]
    fn bit_at(key: &[u8], pos: u16) -> u8 {
        let byte_idx = (pos / 8) as usize;
        let bit_idx = 7 - (pos % 8);
        if byte_idx < key.len() {
            (key[byte_idx] >> bit_idx) & 1
        } else {
            0
        }
    }
}

impl<V> HotTree<V> {
    pub fn get(&self, key: &[u8]) -> Option<&V> {
        if self.root.is_null() {
            return None;
        }

        let mut current = self.root;

        loop {
            if current.is_leaf() {
                let leaf_off = current.leaf_off();
                if self.leaf_key_equals(leaf_off, key) {
                    if std::mem::size_of::<V>() == 0 {
                        return (!current.is_tombstone()).then(|| self.zst_value_ref());
                    }
                    let idx = self.get_leaf_value_idx(leaf_off);
                    return self.values[idx].as_ref();
                }
                return None;
            }

            current = self.node_descend(current.node_off(), key);
        }
    }

    pub fn contains_key(&self, key: &[u8]) -> bool {
        self.get(key).is_some()
    }

    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V> {
        if self.root.is_null() {
            let leaf_ptr = self.store_leaf(key);
            if std::mem::size_of::<V>() == 0 {
                self.zst_values.push(value);
            } else {
                self.values.push(Some(value));
            }
            self.root = leaf_ptr;
            self.count += 1;
            return None;
        }

        if self.root.is_leaf() {
            let leaf_off = self.root.leaf_off();
            if self.leaf_key_equals(leaf_off, key) {
                if std::mem::size_of::<V>() == 0 {
                    if self.root.is_tombstone() {
                        self.root = self.root.without_tombstone();
                        self.zst_values.push(value);
                        self.count += 1;
                        return None;
                    }
                    let old = self
                        .zst_values
                        .pop()
                        .expect("ZST values must track live key count");
                    self.zst_values.push(value);
                    return Some(old);
                }

                let idx = self.get_leaf_value_idx(leaf_off);
                let old = self.values[idx].replace(value);
                if old.is_none() {
                    self.count += 1;
                }
                return old;
            }

            let diff_bit = self
                .first_diff_bit_leaf(leaf_off, key)
                .expect("non-equal keys must have a first differing bit");

            let new_leaf = self.store_leaf(key);
            if std::mem::size_of::<V>() == 0 {
                self.zst_values.push(value);
            } else {
                self.values.push(Some(value));
            }
            self.count += 1;

            let new_bit = Self::bit_at(key, diff_bit);
            let existing_bit = self.bit_at_leaf(leaf_off, diff_bit);
            debug_assert_ne!(new_bit, existing_bit);

            let (left, right) = if new_bit == 0 {
                (new_leaf, self.root)
            } else {
                (self.root, new_leaf)
            };

            self.root = self.create_two_entries_node(diff_bit, left, right);
            return None;
        }

        // Traverse compound nodes to a leaf, recording a stack of (node, entry_idx).
        let mut stack: Vec<InsertFrame> = Vec::with_capacity(64);
        let mut current = self.root;
        while !current.is_leaf() {
            let node_off = current.node_off();
            let entry_idx = self.node_descend_index(node_off, key);
            let child = self.node_entry_ptr(node_off, entry_idx);
            let msb = self.node_most_significant_bit(node_off);
            stack.push(InsertFrame {
                node_off,
                entry_idx,
                msb,
            });
            current = child;
        }

        let leaf_off = current.leaf_off();
        if self.leaf_key_equals(leaf_off, key) {
            if std::mem::size_of::<V>() == 0 {
                if current.is_tombstone() {
                    let parent = stack.last().expect("root is node so stack is non-empty");
                    self.node_set_entry_ptr(
                        parent.node_off,
                        parent.entry_idx,
                        current.without_tombstone(),
                    );
                    self.zst_values.push(value);
                    self.count += 1;
                    return None;
                }
                let old = self
                    .zst_values
                    .pop()
                    .expect("ZST values must track live key count");
                self.zst_values.push(value);
                return Some(old);
            }

            let idx = self.get_leaf_value_idx(leaf_off);
            let old = self.values[idx].replace(value);
            if old.is_none() {
                self.count += 1;
            }
            return old;
        }

        let diff_bit = self
            .first_diff_bit_leaf(leaf_off, key)
            .expect("non-equal keys must have a first differing bit");

        let new_leaf = self.store_leaf(key);
        if std::mem::size_of::<V>() == 0 {
            self.zst_values.push(value);
        } else {
            self.values.push(Some(value));
        }
        self.count += 1;

        let mut insert_depth = 0usize;
        while insert_depth + 1 < stack.len() && diff_bit > stack[insert_depth + 1].msb {
            insert_depth += 1;
        }

        self.insert_at_depth(&stack, insert_depth, key, diff_bit, new_leaf);
        None
    }

    #[inline]
    fn ptr_height(&self, ptr: Ptr) -> u8 {
        if ptr.is_null() || ptr.is_leaf() {
            0
        } else {
            self.nodes.height(ptr.node_off())
        }
    }

    fn create_two_entries_node(&mut self, disc: u16, left: Ptr, right: Ptr) -> Ptr {
        let height = self.ptr_height(left).max(self.ptr_height(right)) + 1;
        let off = self.nodes.alloc_two_entries_node(disc, height, left, right);
        Ptr::node(off)
    }

    #[inline]
    fn hot_tag_for(mapping: DiscriminativeBitsRepresentation) -> u8 {
        let bits = mapping.num_bits();
        match mapping {
            DiscriminativeBitsRepresentation::Single(_) => {
                if bits <= 8 {
                    NODE_HOT_SINGLE_MASK_U8
                } else if bits <= 16 {
                    NODE_HOT_SINGLE_MASK_U16
                } else {
                    NODE_HOT_SINGLE_MASK_U32
                }
            }
            DiscriminativeBitsRepresentation::Multi1(_) => {
                if bits <= 8 {
                    NODE_HOT_MULTI_MASK_8B_U8
                } else if bits <= 16 {
                    NODE_HOT_MULTI_MASK_8B_U16
                } else {
                    NODE_HOT_MULTI_MASK_8B_U32
                }
            }
            DiscriminativeBitsRepresentation::Multi2(_) => NODE_HOT_MULTI_MASK_16B_U16,
            DiscriminativeBitsRepresentation::Multi4(_) => NODE_HOT_MULTI_MASK_32B_U32,
            DiscriminativeBitsRepresentation::Multi8(_) => {
                if bits <= 8 {
                    NODE_HOT_MULTI_MASK_64B_U8
                } else if bits <= 16 {
                    NODE_HOT_MULTI_MASK_64B_U16
                } else {
                    NODE_HOT_MULTI_MASK_64B_U32
                }
            }
        }
    }

    fn export_node_view(
        &self,
        node_off: u64,
        sparse: &mut [u32; MAX_COMPOUND_ENTRIES],
        ptrs: &mut [Ptr; MAX_COMPOUND_ENTRIES],
    ) -> (usize, u8, DiscriminativeBitsRepresentation) {
        let tag = self.nodes.tag(node_off);
        let n = self.nodes.n(node_off);
        let height = self.nodes.height(node_off);

        match tag {
            NODE_TWO_ENTRIES => {
                debug_assert_eq!(n, 2);
                let disc = self.nodes.two_entries_disc(node_off);
                let mapping = DiscriminativeBitsRepresentation::build_minimal(&[disc]);
                sparse[0] = 0;
                sparse[1] = 1;
                ptrs[0] = self.nodes.two_entries_ptr_at(node_off, 0);
                ptrs[1] = self.nodes.two_entries_ptr_at(node_off, 1);
                (2, height, mapping)
            }
            t if hot_is_hot_node(t) => {
                let mapping = self.nodes.hot_mapping(node_off);
                for i in 0..n {
                    sparse[i] = self.nodes.hot_partial_key_u32_at(node_off, i);
                    ptrs[i] = self.nodes.hot_entry_ptr_at(node_off, i);
                }
                (n, height, mapping)
            }
            other => panic!("unknown node tag {other}"),
        }
    }

    fn view_affected_range(
        mapping: DiscriminativeBitsRepresentation,
        sparse: &[u32; MAX_COMPOUND_ENTRIES],
        n: usize,
        entry_idx: usize,
        disc: u16,
    ) -> (usize, usize, u32, u32) {
        debug_assert!(entry_idx < n);
        let prefix_bits = mapping.prefix_mask_u32(disc);
        let subtree_prefix = sparse[entry_idx] & prefix_bits;

        let mut first: Option<usize> = None;
        let mut count: usize = 0;
        let mut saw_gap = false;
        for i in 0..n {
            let m = (sparse[i] & prefix_bits) == subtree_prefix;
            if m {
                if saw_gap {
                    panic!("non-contiguous affected subtree in HOT node");
                }
                if first.is_none() {
                    first = Some(i);
                }
                count += 1;
            } else if first.is_some() {
                saw_gap = true;
            }
        }

        (
            first.expect("affected subtree must be non-empty"),
            count,
            prefix_bits,
            subtree_prefix,
        )
    }

    #[inline]
    fn recode_insert_bit(old: u32, new_pos: u32) -> u32 {
        debug_assert!(new_pos <= 31);
        if new_pos == 0 {
            old << 1
        } else if new_pos == 31 {
            (old & ((1u32 << 31) - 1)) | ((old & (1u32 << 31)) << 1)
        } else {
            let low_mask = (1u32 << new_pos) - 1;
            let low = old & low_mask;
            let high = old & !low_mask;
            low | (high << 1)
        }
    }

    #[inline]
    fn relevant_bits_for_range(
        sparse: &[u32; MAX_COMPOUND_ENTRIES],
        start: usize,
        count: usize,
    ) -> u32 {
        if count <= 1 {
            return 0;
        }
        let end = start + count;
        let mut relevant = 0u32;
        for i in (start + 1)..end {
            relevant |= sparse[i] & !sparse[i - 1];
        }
        relevant
    }

    #[inline]
    fn determine_value_of_discriminating_bit(
        sparse: &[u32; MAX_COMPOUND_ENTRIES],
        entry_idx: usize,
        n: usize,
    ) -> u8 {
        debug_assert!(entry_idx < n);
        debug_assert!(n >= 2);
        if entry_idx == 0 {
            0
        } else if entry_idx + 1 == n {
            1
        } else {
            let left = sparse[entry_idx - 1] & sparse[entry_idx];
            let right = sparse[entry_idx] & sparse[entry_idx + 1];
            u8::from(left >= right)
        }
    }

    #[inline]
    fn relevant_bits_for_all_except_one(
        sparse: &[u32; MAX_COMPOUND_ENTRIES],
        n: usize,
        index_to_remove: usize,
    ) -> u32 {
        debug_assert!(n >= 2);
        debug_assert!(index_to_remove < n);
        if n <= 2 {
            return 0;
        }
        let disc_value = Self::determine_value_of_discriminating_bit(sparse, index_to_remove, n);
        let first_range_len = index_to_remove + usize::from(disc_value == 0);
        let left = Self::relevant_bits_for_range(sparse, 0, first_range_len);
        let right = Self::relevant_bits_for_range(sparse, first_range_len, n - first_range_len);
        left | right
    }

    fn node_remove_entry(&mut self, node_off: u64, entry_idx: usize) -> Ptr {
        let tag = self.nodes.tag(node_off);
        match tag {
            NODE_TWO_ENTRIES => {
                debug_assert!(entry_idx < 2);
                self.nodes.two_entries_ptr_at(node_off, 1 - entry_idx)
            }
            t if hot_is_hot_node(t) => {
                let mut sparse = [0u32; MAX_COMPOUND_ENTRIES];
                let mut ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
                let (n, _height, mapping) = self.export_node_view(node_off, &mut sparse, &mut ptrs);
                debug_assert!(n >= 3);
                debug_assert!(entry_idx < n);

                let k = mapping.num_bits() as usize;
                debug_assert!(k <= 32);
                let mut disc_bits: Vec<u16> = Vec::with_capacity(k);
                mapping.discriminative_bits(&mut disc_bits);
                disc_bits.sort_unstable();
                disc_bits.dedup();
                debug_assert_eq!(disc_bits.len(), k);

                let disc_value =
                    Self::determine_value_of_discriminating_bit(&sparse, entry_idx, n) as usize;
                let left_mask = sparse[entry_idx - disc_value];
                let right_mask = sparse[entry_idx + (1 - disc_value)];
                let expected_prefix = left_mask & right_mask;
                let diff = expected_prefix ^ right_mask;
                debug_assert_ne!(diff, 0);
                let disc_mask = diff & diff.wrapping_neg();
                let disc_dense_pos = disc_mask.trailing_zeros() as usize;
                debug_assert!(disc_dense_pos < k);
                let disc_abs = disc_bits[(k - 1) - disc_dense_pos];

                let (range_first, range_count, _prefix_bits, _subtree_prefix) =
                    Self::view_affected_range(mapping, &sparse, n, entry_idx, disc_abs);
                debug_assert!(range_count >= 2);
                let range_last = range_first + range_count;
                if disc_value == 0 {
                    debug_assert_eq!(entry_idx, range_first);
                } else {
                    debug_assert_eq!(entry_idx, range_last - 1);
                }

                let compression_mask =
                    Self::relevant_bits_for_all_except_one(&sparse, n, entry_idx);
                debug_assert_ne!(compression_mask, 0);

                let mut new_disc_bits: Vec<u16> =
                    Vec::with_capacity(compression_mask.count_ones() as usize);
                for (i, &abs_bit) in disc_bits.iter().enumerate() {
                    let dense_pos = (k - 1) - i;
                    if (compression_mask & (1u32 << dense_pos)) != 0 {
                        new_disc_bits.push(abs_bit);
                    }
                }
                new_disc_bits.sort_unstable();
                new_disc_bits.dedup();

                let new_mapping = DiscriminativeBitsRepresentation::build_minimal(&new_disc_bits);

                let new_n = n - 1;
                let mut new_sparse = [0u32; MAX_COMPOUND_ENTRIES];
                let mut new_ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
                for i in 0..entry_idx {
                    new_sparse[i] = pext_u64(sparse[i] as u64, compression_mask as u64) as u32;
                    new_ptrs[i] = ptrs[i];
                }
                for i in (entry_idx + 1)..n {
                    let j = i - 1;
                    new_sparse[j] = pext_u64(sparse[i] as u64, compression_mask as u64) as u32;
                    new_ptrs[j] = ptrs[i];
                }

                let compressed_disc_mask =
                    pext_u64(disc_mask as u64, compression_mask as u64) as u32;
                if compressed_disc_mask != 0 {
                    let delete_unused_mask = !compressed_disc_mask;
                    for orig_i in range_first..range_last {
                        if orig_i == entry_idx {
                            continue;
                        }
                        let new_i = if orig_i < entry_idx {
                            orig_i
                        } else {
                            orig_i - 1
                        };
                        debug_assert!(new_i < new_n);
                        new_sparse[new_i] &= delete_unused_mask;
                    }
                }

                // This is important for lookup integrity: the first mask must always be zero.
                new_sparse[0] = 0;

                if new_n == 2 {
                    debug_assert_eq!(new_disc_bits.len(), new_mapping.num_bits() as usize);
                    let rel = new_sparse[1];
                    debug_assert_ne!(rel, 0);
                    debug_assert_eq!(rel.count_ones(), 1);
                    let dense_pos = rel.trailing_zeros() as usize;
                    debug_assert!(dense_pos < new_disc_bits.len());
                    let disc = new_disc_bits[(new_disc_bits.len() - 1) - dense_pos];
                    return self.create_two_entries_node(disc, new_ptrs[0], new_ptrs[1]);
                }

                let mut height = 0u8;
                for i in 0..new_n {
                    height = height.max(self.ptr_height(new_ptrs[i]));
                }
                height += 1;

                let tag = Self::hot_tag_for(new_mapping);
                let off = self.nodes.alloc_hot_node(
                    tag,
                    height,
                    new_mapping,
                    &new_sparse[..new_n],
                    &new_ptrs[..new_n],
                );
                Ptr::node(off)
            }
            other => panic!("node_remove_entry: unknown tag {other}"),
        }
    }

    fn build_subtree_from_range(
        &mut self,
        mapping: DiscriminativeBitsRepresentation,
        sparse: &[u32; MAX_COMPOUND_ENTRIES],
        ptrs: &[Ptr; MAX_COMPOUND_ENTRIES],
        start: usize,
        count: usize,
    ) -> Ptr {
        debug_assert!(count >= 1);
        if count == 1 {
            return ptrs[start];
        }

        let k = mapping.num_bits() as usize;
        debug_assert!(k <= 32);

        let mut disc_bits: Vec<u16> = Vec::with_capacity(k);
        mapping.discriminative_bits(&mut disc_bits);
        disc_bits.sort_unstable();
        disc_bits.dedup();
        debug_assert_eq!(disc_bits.len(), k);

        let relevant = Self::relevant_bits_for_range(sparse, start, count);
        debug_assert_ne!(relevant, 0);

        if count == 2 {
            debug_assert_eq!(relevant.count_ones(), 1);
            let dense_pos = relevant.trailing_zeros() as usize;
            let idx_in_order = (k - 1) - dense_pos;
            let disc = disc_bits[idx_in_order];
            let left = ptrs[start];
            let right = ptrs[start + 1];
            return self.create_two_entries_node(disc, left, right);
        }

        let mut new_disc_bits: Vec<u16> = Vec::with_capacity(relevant.count_ones() as usize);
        for (i, &abs_bit) in disc_bits.iter().enumerate() {
            let dense_pos = (k - 1) - i;
            if (relevant & (1u32 << dense_pos)) != 0 {
                new_disc_bits.push(abs_bit);
            }
        }
        new_disc_bits.sort_unstable();
        new_disc_bits.dedup();
        debug_assert_eq!(new_disc_bits.len(), relevant.count_ones() as usize);
        debug_assert!(new_disc_bits.len() >= 2);

        let new_mapping = DiscriminativeBitsRepresentation::build_minimal(&new_disc_bits);
        let mut new_sparse = [0u32; MAX_COMPOUND_ENTRIES];
        let mut new_ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];

        for i in 0..count {
            let pk = sparse[start + i];
            new_sparse[i] = pext_u64(pk as u64, relevant as u64) as u32;
            new_ptrs[i] = ptrs[start + i];
        }
        new_sparse[0] = 0;

        let mut height = 0u8;
        for i in 0..count {
            height = height.max(self.ptr_height(new_ptrs[i]));
        }
        height += 1;

        let tag = Self::hot_tag_for(new_mapping);
        let off = self.nodes.alloc_hot_node(
            tag,
            height,
            new_mapping,
            &new_sparse[..count],
            &new_ptrs[..count],
        );
        Ptr::node(off)
    }

    fn node_add_entry_by_bit(
        &mut self,
        node_off: u64,
        entry_idx: usize,
        disc: u16,
        new_bit: u8,
        new_ptr: Ptr,
        replace_existing_ptr: Option<Ptr>,
    ) -> Ptr {
        let mut sparse = [0u32; MAX_COMPOUND_ENTRIES];
        let mut ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
        let (n, _height, mapping) = self.export_node_view(node_off, &mut sparse, &mut ptrs);
        debug_assert!(n < MAX_COMPOUND_ENTRIES);
        debug_assert!(entry_idx < n);

        if let Some(p) = replace_existing_ptr {
            ptrs[entry_idx] = p;
        }

        let (first, count, prefix_bits, subtree_prefix) =
            Self::view_affected_range(mapping, &sparse, n, entry_idx, disc);
        debug_assert!(count >= 1);
        let insert_idx = first + (new_bit as usize) * count;

        let mut disc_bits: Vec<u16> = Vec::with_capacity(mapping.num_bits() as usize + 1);
        mapping.discriminative_bits(&mut disc_bits);
        disc_bits.push(disc);
        disc_bits.sort_unstable();
        disc_bits.dedup();

        let new_mapping = DiscriminativeBitsRepresentation::build_minimal(&disc_bits);
        let old_k = mapping.num_bits() as u32;
        let new_k = new_mapping.num_bits() as u32;
        debug_assert!(new_k == old_k || new_k == old_k + 1);
        debug_assert!(new_k <= 32);

        let more_significant = prefix_bits.count_ones();
        debug_assert!(more_significant <= old_k);
        let (recode_pos, add_mask) = if new_k == old_k + 1 {
            let new_pos = old_k - more_significant;
            debug_assert!(new_pos <= old_k);
            (Some(new_pos), 1u32 << new_pos)
        } else {
            debug_assert!(more_significant < old_k);
            let disc_pos = old_k - more_significant - 1;
            (None, 1u32 << disc_pos)
        };

        let mut new_sparse = [0u32; MAX_COMPOUND_ENTRIES];
        let mut new_ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];

        let mut old_i = 0usize;
        for new_i in 0..(n + 1) {
            if new_i == insert_idx {
                let mut pk = match recode_pos {
                    Some(pos) => Self::recode_insert_bit(subtree_prefix, pos),
                    None => subtree_prefix,
                };
                if new_bit != 0 {
                    pk |= add_mask;
                }
                new_sparse[new_i] = pk;
                new_ptrs[new_i] = new_ptr;
                continue;
            }

            let i = old_i;
            old_i += 1;
            let mut pk = match recode_pos {
                Some(pos) => Self::recode_insert_bit(sparse[i], pos),
                None => sparse[i],
            };

            // Existing entries in the affected subtree all have the opposite discriminative bit value.
            if (i >= first) && (i < first + count) && (new_bit == 0) {
                pk |= add_mask;
            }

            new_sparse[new_i] = pk;
            new_ptrs[new_i] = ptrs[i];
        }
        debug_assert_eq!(old_i, n);
        new_sparse[0] = 0;

        let mut height = 0u8;
        for i in 0..(n + 1) {
            height = height.max(self.ptr_height(new_ptrs[i]));
        }
        height += 1;

        let tag = Self::hot_tag_for(new_mapping);
        let off = self.nodes.alloc_hot_node(
            tag,
            height,
            new_mapping,
            &new_sparse[..(n + 1)],
            &new_ptrs[..(n + 1)],
        );
        Ptr::node(off)
    }

    fn node_pull_up_split_entry(
        &mut self,
        node_off: u64,
        entry_idx: usize,
        disc: u16,
        left: Ptr,
        right: Ptr,
    ) -> Ptr {
        // Split exactly this entry: existing becomes the left child (bit=0), new becomes right (bit=1).
        self.node_add_entry_by_bit(node_off, entry_idx, disc, 1u8, right, Some(left))
    }

    fn insert_into_nonfull_subtree(
        &mut self,
        subtree: Ptr,
        key: &[u8],
        disc: u16,
        new_ptr: Ptr,
    ) -> Ptr {
        let new_bit = Self::bit_at(key, disc);
        if subtree.is_leaf() {
            let (left, right) = if new_bit == 0 {
                (new_ptr, subtree)
            } else {
                (subtree, new_ptr)
            };
            return self.create_two_entries_node(disc, left, right);
        }

        let off = subtree.node_off();
        let entry_idx = self.node_descend_index(off, key);
        let out = self.node_add_entry_by_bit(off, entry_idx, disc, new_bit, new_ptr, None);
        self.nodes.free_node(off);
        out
    }

    #[inline]
    fn replace_ptr_at_depth(&mut self, stack: &[InsertFrame], depth: usize, ptr: Ptr) {
        if depth == 0 {
            self.root = ptr;
        } else {
            let parent = &stack[depth - 1];
            self.node_set_entry_ptr(parent.node_off, parent.entry_idx, ptr);
        }
    }

    fn split_full_node_for_insert(
        &mut self,
        node_off: u64,
        key: &[u8],
        disc: u16,
        new_leaf: Ptr,
    ) -> BiNodeSplit {
        let mut sparse = [0u32; MAX_COMPOUND_ENTRIES];
        let mut ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
        let (n, _height, mapping) = self.export_node_view(node_off, &mut sparse, &mut ptrs);
        debug_assert_eq!(n, MAX_COMPOUND_ENTRIES);

        let root_disc = mapping.most_significant_bit();
        debug_assert!(disc > root_disc);

        let root_dense_pos = (mapping.num_bits() - 1) as u32;
        let root_dense_mask = 1u32 << root_dense_pos;

        let mut left_count = 0usize;
        while left_count < n && (sparse[left_count] & root_dense_mask) == 0 {
            left_count += 1;
        }
        debug_assert!(left_count > 0 && left_count < n);
        let right_count = n - left_count;

        let mut left_ptr = self.build_subtree_from_range(mapping, &sparse, &ptrs, 0, left_count);
        let mut right_ptr =
            self.build_subtree_from_range(mapping, &sparse, &ptrs, left_count, right_count);

        let which_side = Self::bit_at(key, root_disc);
        if which_side == 0 {
            left_ptr = self.insert_into_nonfull_subtree(left_ptr, key, disc, new_leaf);
        } else {
            right_ptr = self.insert_into_nonfull_subtree(right_ptr, key, disc, new_leaf);
        }

        let height = self.ptr_height(left_ptr).max(self.ptr_height(right_ptr)) + 1;
        BiNodeSplit {
            disc: root_disc,
            height,
            left: left_ptr,
            right: right_ptr,
        }
    }

    fn split_full_node_for_integration(
        &mut self,
        node_off: u64,
        entry_idx: usize,
        disc: u16,
        left: Ptr,
        right: Ptr,
    ) -> BiNodeSplit {
        let mut sparse = [0u32; MAX_COMPOUND_ENTRIES];
        let mut ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
        let (n, _height, mapping) = self.export_node_view(node_off, &mut sparse, &mut ptrs);
        debug_assert_eq!(n, MAX_COMPOUND_ENTRIES);
        debug_assert!(entry_idx < n);

        let root_disc = mapping.most_significant_bit();
        debug_assert!(disc > root_disc);

        let root_dense_pos = (mapping.num_bits() - 1) as u32;
        let root_dense_mask = 1u32 << root_dense_pos;

        let mut left_count = 0usize;
        while left_count < n && (sparse[left_count] & root_dense_mask) == 0 {
            left_count += 1;
        }
        debug_assert!(left_count > 0 && left_count < n);
        let right_count = n - left_count;

        let (affected_start, affected_count, other_start, other_count, affected_side, local_idx) =
            if entry_idx < left_count {
                (0, left_count, left_count, right_count, 0u8, entry_idx)
            } else {
                (
                    left_count,
                    right_count,
                    0,
                    left_count,
                    1u8,
                    entry_idx - left_count,
                )
            };

        let other_ptr =
            self.build_subtree_from_range(mapping, &sparse, &ptrs, other_start, other_count);

        let affected_ptr = if affected_count == 1 {
            self.create_two_entries_node(disc, left, right)
        } else {
            let subtree = self.build_subtree_from_range(
                mapping,
                &sparse,
                &ptrs,
                affected_start,
                affected_count,
            );
            let off = subtree.node_off();
            let new_subtree = self.node_pull_up_split_entry(off, local_idx, disc, left, right);
            self.nodes.free_node(off);
            new_subtree
        };

        let (left_ptr, right_ptr) = if affected_side == 0 {
            (affected_ptr, other_ptr)
        } else {
            (other_ptr, affected_ptr)
        };

        let height = self.ptr_height(left_ptr).max(self.ptr_height(right_ptr)) + 1;
        BiNodeSplit {
            disc: root_disc,
            height,
            left: left_ptr,
            right: right_ptr,
        }
    }

    fn integrate_binode(
        &mut self,
        stack: &[InsertFrame],
        current_depth: usize,
        split: BiNodeSplit,
    ) {
        if current_depth == 0 {
            self.root = self.create_two_entries_node(split.disc, split.left, split.right);
            return;
        }

        let parent_depth = current_depth - 1;
        let parent_off = stack[parent_depth].node_off;
        let parent_entry_idx = stack[parent_depth].entry_idx;

        let parent_height = self.nodes.height(parent_off);
        if parent_height > split.height {
            let intermediate = self.create_two_entries_node(split.disc, split.left, split.right);
            self.node_set_entry_ptr(parent_off, parent_entry_idx, intermediate);
            return;
        }

        self.parent_pull_up(stack, parent_depth, split);
    }

    fn parent_pull_up(&mut self, stack: &[InsertFrame], parent_depth: usize, split: BiNodeSplit) {
        let parent_off = stack[parent_depth].node_off;
        let parent_n = self.node_entry_count(parent_off);
        debug_assert!(parent_n >= 2 && parent_n <= MAX_COMPOUND_ENTRIES);
        debug_assert!(split.height == self.nodes.height(parent_off));

        let entry_idx = stack[parent_depth].entry_idx;
        debug_assert!(entry_idx < parent_n);

        if parent_n < MAX_COMPOUND_ENTRIES {
            let new_parent_ptr = self.node_pull_up_split_entry(
                parent_off,
                entry_idx,
                split.disc,
                split.left,
                split.right,
            );

            self.replace_ptr_at_depth(stack, parent_depth, new_parent_ptr);
            self.nodes.free_node(parent_off);
            return;
        }

        let new_split = self.split_full_node_for_integration(
            parent_off,
            entry_idx,
            split.disc,
            split.left,
            split.right,
        );
        self.nodes.free_node(parent_off);
        self.integrate_binode(stack, parent_depth, new_split);
    }

    fn insert_at_depth(
        &mut self,
        stack: &[InsertFrame],
        insert_depth: usize,
        key: &[u8],
        disc: u16,
        new_leaf: Ptr,
    ) {
        debug_assert!(insert_depth < stack.len());
        let frame = &stack[insert_depth];
        let node_off = frame.node_off;
        let node_n = self.node_entry_count(node_off);
        let node_height = self.nodes.height(node_off);
        let node_msb = self.node_most_significant_bit(node_off);

        let mut sparse = [0u32; MAX_COMPOUND_ENTRIES];
        let mut ptrs = [Ptr::NULL; MAX_COMPOUND_ENTRIES];
        let (n, _h, mapping) = self.export_node_view(node_off, &mut sparse, &mut ptrs);
        debug_assert_eq!(n, node_n);

        let (first_idx, count, _prefix_bits, _subtree_prefix) =
            Self::view_affected_range(mapping, &sparse, n, frame.entry_idx, disc);
        debug_assert!(first_idx < node_n);

        let new_bit = Self::bit_at(key, disc);

        // Leaf-node pushdown: replace the leaf entry with a new 2-entry node if there is vertical room.
        if count == 1 {
            let entry_ptr = ptrs[first_idx];
            if entry_ptr.is_leaf() && node_height > 1 {
                let (left, right) = if new_bit == 0 {
                    (new_leaf, entry_ptr)
                } else {
                    (entry_ptr, new_leaf)
                };
                let pushed = self.create_two_entries_node(disc, left, right);
                self.node_set_entry_ptr(node_off, first_idx, pushed);
                return;
            }

            let is_leaf_entry = insert_depth + 1 == stack.len();
            if !is_leaf_entry {
                // Boundary-node case: insert into the child partition as a new root (or integrate if full).
                debug_assert!(!entry_ptr.is_leaf());
                debug_assert_eq!(first_idx, frame.entry_idx);
                let child_off = entry_ptr.node_off();
                let child_n = self.node_entry_count(child_off);
                if child_n < MAX_COMPOUND_ENTRIES {
                    let child_entry_idx = stack[insert_depth + 1].entry_idx;
                    let new_child_ptr = self.node_add_entry_by_bit(
                        child_off,
                        child_entry_idx,
                        disc,
                        new_bit,
                        new_leaf,
                        None,
                    );
                    self.node_set_entry_ptr(node_off, first_idx, new_child_ptr);
                    self.nodes.free_node(child_off);
                } else {
                    let child_height = self.nodes.height(child_off);
                    let (left, right) = if new_bit == 0 {
                        (new_leaf, entry_ptr)
                    } else {
                        (entry_ptr, new_leaf)
                    };
                    let split = BiNodeSplit {
                        disc,
                        height: child_height + 1,
                        left,
                        right,
                    };
                    self.integrate_binode(stack, insert_depth + 1, split);
                }
                return;
            }
        }

        if node_n < MAX_COMPOUND_ENTRIES {
            let new_node_ptr = self.node_add_entry_by_bit(
                node_off,
                frame.entry_idx,
                disc,
                new_bit,
                new_leaf,
                None,
            );
            self.replace_ptr_at_depth(stack, insert_depth, new_node_ptr);
            self.nodes.free_node(node_off);
            return;
        }

        // Node full: either split on the node's root discriminator or create a new partition root above it.
        debug_assert_eq!(node_n, MAX_COMPOUND_ENTRIES);
        debug_assert_ne!(disc, node_msb);
        if disc > node_msb {
            let split = self.split_full_node_for_insert(node_off, key, disc, new_leaf);
            self.nodes.free_node(node_off);
            self.integrate_binode(stack, insert_depth, split);
        } else {
            let node_ptr = Ptr::node(node_off);
            let node_h = self.nodes.height(node_off);
            let (left, right) = if new_bit == 0 {
                (new_leaf, node_ptr)
            } else {
                (node_ptr, new_leaf)
            };
            let split = BiNodeSplit {
                disc,
                height: node_h + 1,
                left,
                right,
            };
            self.integrate_binode(stack, insert_depth, split);
        }
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<V> {
        if self.root.is_null() {
            return None;
        }

        // Descend to a leaf, recording (node_off, entry_idx) along the path.
        let mut stack: Vec<(u64, usize)> = Vec::with_capacity(64);
        let mut current = self.root;
        while !current.is_leaf() {
            let node_off = current.node_off();
            let entry_idx = self.node_descend_index(node_off, key);
            stack.push((node_off, entry_idx));
            current = self.node_entry_ptr(node_off, entry_idx);
        }

        if current.is_tombstone() {
            return None;
        }

        let leaf_off = current.leaf_off();
        if !self.leaf_key_equals(leaf_off, key) {
            return None;
        }

        let old = if std::mem::size_of::<V>() == 0 {
            Some(
                self.zst_values
                    .pop()
                    .expect("ZST values must track live key count"),
            )
        } else {
            let idx = self.get_leaf_value_idx(leaf_off);
            self.values[idx].take()
        }?;

        self.count -= 1;

        // Removing the root leaf.
        if stack.is_empty() {
            self.root = Ptr::NULL;
            return Some(old);
        }

        // Remove entry from its parent (allocates a new parent node or collapses to the sibling).
        let (parent_off, parent_entry_idx) = stack.pop().expect("stack non-empty");
        let mut replacement = self.node_remove_entry(parent_off, parent_entry_idx);
        self.nodes.free_node(parent_off);

        // Parent was root.
        if stack.is_empty() {
            self.root = replacement;
            return Some(old);
        }

        // Propagate the replacement upward by updating child pointers in-place and fixing heights.
        while let Some((node_off, entry_idx)) = stack.pop() {
            self.node_set_entry_ptr(node_off, entry_idx, replacement);
            let old_h = self.nodes.height(node_off);
            let n = self.node_entry_count(node_off);
            let mut h = 0u8;
            for i in 0..n {
                h = h.max(self.ptr_height(self.node_entry_ptr(node_off, i)));
            }
            h = h.saturating_add(1);
            self.nodes.set_height(node_off, h);
            if h == old_h {
                break;
            }
            replacement = Ptr::node(node_off);
        }

        Some(old)
    }

    pub fn iter(&self) -> Iter<'_, V> {
        let mut stack = Vec::new();
        if !self.root.is_null() {
            stack.push(self.root);
        }
        Iter { tree: self, stack }
    }
}

impl<V> Default for HotTree<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> Clone for HotTree<V> {
    fn clone(&self) -> Self {
        Self {
            prefix_pool: self.prefix_pool.clone(),
            prefix_offsets: self.prefix_offsets.clone(),
            prefix_hash: self.prefix_hash.clone(),
            leaves: self.leaves.clone(),
            values: self.values.clone(),
            zst_values: self.zst_values.clone(),
            nodes: self.nodes.clone(),
            root: self.root,
            count: self.count,
            _marker: PhantomData,
        }
    }
}

impl<V: std::fmt::Debug + Clone> std::fmt::Debug for HotTree<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

pub struct Iter<'a, V> {
    tree: &'a HotTree<V>,
    stack: Vec<Ptr>,
}

impl<'a, V> Iterator for Iter<'a, V> {
    type Item = (Vec<u8>, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(ptr) = self.stack.pop() {
            if ptr.is_null() {
                continue;
            }

            if ptr.is_leaf() {
                let leaf_off = ptr.leaf_off();
                if std::mem::size_of::<V>() == 0 {
                    if ptr.is_tombstone() {
                        continue;
                    }
                    let key = self.tree.get_leaf_key(leaf_off);
                    return Some((key, self.tree.zst_value_ref()));
                }

                let idx = self.tree.get_leaf_value_idx(leaf_off);
                if let Some(ref value) = self.tree.values[idx] {
                    let key = self.tree.get_leaf_key(leaf_off);
                    return Some((key, value));
                }
                continue;
            }

            let node_off = ptr.node_off();
            let n = self.tree.node_entry_count(node_off);
            for i in (0..n).rev() {
                self.stack.push(self.tree.node_entry_ptr(node_off, i));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"hello", 1);
        t.insert(b"world", 2);
        assert_eq!(t.get(b"hello"), Some(&1));
        assert_eq!(t.get(b"world"), Some(&2));
        assert_eq!(t.get(b"missing"), None);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn test_update() {
        let mut t: HotTree<u64> = HotTree::new();
        assert_eq!(t.insert(b"key", 1), None);
        assert_eq!(t.insert(b"key", 2), Some(1));
        assert_eq!(t.get(b"key"), Some(&2));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn test_remove() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        t.insert(b"b", 2);
        t.insert(b"c", 3);

        assert_eq!(t.remove(b"b"), Some(2));
        assert_eq!(t.get(b"b"), None);
        assert_eq!(t.len(), 2);
        assert_eq!(t.get(b"a"), Some(&1));
        assert_eq!(t.get(b"c"), Some(&3));

        // Reinserting a removed key should increase length.
        assert_eq!(t.insert(b"b", 4), None);
        assert_eq!(t.get(b"b"), Some(&4));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn test_zst_remove_and_reinsert() {
        let mut t: HotTree<()> = HotTree::new();
        assert_eq!(t.insert(b"a", ()), None);
        assert_eq!(t.insert(b"b", ()), None);
        assert_eq!(t.insert(b"c", ()), None);
        assert_eq!(t.len(), 3);
        assert_eq!(t.get(b"a"), Some(&()));
        assert_eq!(t.get(b"b"), Some(&()));
        assert_eq!(t.get(b"c"), Some(&()));

        assert_eq!(t.remove(b"b"), Some(()));
        assert_eq!(t.get(b"b"), None);
        assert_eq!(t.get(b"a"), Some(&()));
        assert_eq!(t.get(b"c"), Some(&()));
        assert_eq!(t.len(), 2);

        // Reinsertion should clear the tombstone.
        assert_eq!(t.insert(b"b", ()), None);
        assert_eq!(t.get(b"b"), Some(&()));
        assert_eq!(t.len(), 3);

        // Updating an existing key should return Some(()) and keep length stable.
        assert_eq!(t.insert(b"b", ()), Some(()));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn test_many() {
        let mut t: HotTree<u64> = HotTree::new();
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }
        assert_eq!(t.len(), 1000);
        for i in 0..1000u64 {
            let key = format!("key{:05}", i);
            assert_eq!(t.get(key.as_bytes()), Some(&i), "Failed at {}", i);
        }
    }

    #[test]
    fn test_prefix_compression() {
        let mut t: HotTree<u64> = HotTree::new();
        // URLs with shared prefix
        t.insert(b"https://example.com/page1", 1);
        t.insert(b"https://example.com/page2", 2);
        t.insert(b"https://example.com/page3", 3);
        t.insert(b"https://other.com/page1", 4);

        assert_eq!(t.get(b"https://example.com/page1"), Some(&1));
        assert_eq!(t.get(b"https://example.com/page2"), Some(&2));
        assert_eq!(t.get(b"https://example.com/page3"), Some(&3));
        assert_eq!(t.get(b"https://other.com/page1"), Some(&4));

        // Check that prefixes were learned
        assert!(t.prefix_offsets.len() > 1, "Should have learned prefixes");
    }

    #[test]
    fn test_iter() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"b", 2);
        t.insert(b"a", 1);
        t.insert(b"c", 3);

        let pairs: Vec<_> = t.iter().collect();
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], (b"a".to_vec(), &1));
        assert_eq!(pairs[1], (b"b".to_vec(), &2));
        assert_eq!(pairs[2], (b"c".to_vec(), &3));
    }

    #[test]
    fn test_iter_sorted_random() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        use std::collections::BTreeMap;

        let mut rng = StdRng::seed_from_u64(1);
        let mut t: HotTree<u64> = HotTree::new();
        let mut m: BTreeMap<Vec<u8>, u64> = BTreeMap::new();

        for _ in 0..2000 {
            let len = rng.gen_range(0..33);
            let mut key = vec![0u8; len];
            for b in &mut key {
                // Avoid 0x00 for now: this implementation treats missing bytes as 0
                // at the bit level, so keys that differ only by trailing 0x00 bytes
                // are not distinguishable.
                *b = rng.gen_range(1..=255);
            }
            let v: u64 = rng.gen();
            assert_eq!(t.insert(&key, v), m.insert(key, v));
        }

        let got: Vec<(Vec<u8>, u64)> = t.iter().map(|(k, v)| (k, *v)).collect();
        let expected: Vec<(Vec<u8>, u64)> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn test_randomized_insert_remove_get() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        use std::collections::BTreeMap;

        let mut rng = StdRng::seed_from_u64(2);
        let mut t: HotTree<u64> = HotTree::new();
        let mut m: BTreeMap<Vec<u8>, u64> = BTreeMap::new();

        for _ in 0..50_000 {
            let op = rng.gen_range(0..100);
            let len = rng.gen_range(0..33);
            let mut key = vec![0u8; len];
            for b in &mut key {
                *b = rng.gen_range(1..=255);
            }

            match op {
                0..=49 => {
                    let v: u64 = rng.gen();
                    assert_eq!(t.insert(&key, v), m.insert(key, v));
                }
                50..=74 => {
                    assert_eq!(t.remove(&key), m.remove(&key));
                }
                _ => {
                    assert_eq!(t.get(&key).copied(), m.get(&key).copied());
                }
            }
        }

        assert_eq!(t.len(), m.len());
        let got: Vec<(Vec<u8>, u64)> = t.iter().map(|(k, v)| (k, *v)).collect();
        let expected: Vec<(Vec<u8>, u64)> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn test_empty_key() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"", 42);
        assert_eq!(t.get(b""), Some(&42));
    }

    #[test]
    fn test_contains_key() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"exists", 1);
        assert!(t.contains_key(b"exists"));
        assert!(!t.contains_key(b"missing"));
    }

    #[test]
    fn test_clone() {
        let mut t: HotTree<u64> = HotTree::new();
        t.insert(b"a", 1);
        t.insert(b"b", 2);
        let t2 = t.clone();
        assert_eq!(t2.get(b"a"), Some(&1));
        assert_eq!(t2.get(b"b"), Some(&2));
    }

    #[test]
    fn test_compact() {
        let mut t: HotTree<u64> = HotTree::new();
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            t.insert(key.as_bytes(), i);
        }

        // Verify before compaction
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            assert_eq!(
                t.get(key.as_bytes()),
                Some(&i),
                "Failed before compact at {}",
                i
            );
        }

        // Compact the tree
        let _ = t.compact();

        // Verify after compaction
        for i in 0..100u64 {
            let key = format!("key{:05}", i);
            assert_eq!(
                t.get(key.as_bytes()),
                Some(&i),
                "Failed after compact at {}",
                i
            );
        }

        // Test iterator after compaction
        let count = t.iter().count();
        assert_eq!(count, 100);
    }
}

#[cfg(test)]
mod proptests;
