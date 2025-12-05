# memkv: Memory-Efficient Key-Value Storage Design Document

## Overview

This library provides memory-efficient storage for string keys with arbitrary values, designed to support billions of keys in memory with minimal overhead.

---

## Comprehensive Benchmark Results (December 2024)

### Dataset: 9.5 Million Real-World URLs (467 MB raw data)

| Implementation | Memory | Overhead/Key | Insert ops/s | Lookup ops/s | Notes |
|---------------|--------|--------------|--------------|--------------|-------|
| **FrozenLayer (FST)** | **320 MB** | **-16.2 bytes** | 661K | 2.6M | **COMPRESSION!** Immutable |
| **memkv::FastArt** | **998 MB** | **58.4 bytes** | **5.1M** | **9.9M** | **NEW BEST MUTABLE ART** |
| libart (C) | 1,123 MB | 72.2 bytes | 4.9M | 9.6M | C reference impl |
| std::BTreeMap | 1,145 MB | 74.6 bytes | 3.3M | 7.8M | Standard library |
| memkv::ArenaArt | 1,449 MB | 108.1 bytes | 3.1M | 10.7M | Arena-based |
| art-tree (crate) | 1,961 MB | 164.4 bytes | 3.8M | - | Variable keys |
| blart (crate) | 3,286 MB | 310.3 bytes | 3.0M | - | Fixed 256B keys |
| art (crate) | 6,953 MB | 713.9 bytes | 1.0M | - | Fixed 256B keys |
| rart (crate) | 9,875 MB | 1,035.6 bytes | 1.6M | - | SIMD, fixed keys |

### Key Findings

1. **FST achieves NEGATIVE overhead** (-16.2 bytes/key = compression)
   - 2.4x compression ratio vs raw data
   - 194 MB FST for 467 MB of keys
   - Best for immutable/frozen data

2. **FastArt beats libart AND BTreeMap!**
   - **58.4 bytes overhead** (19% less than libart's 72.2)
   - Fastest mutable insert: 5.1M ops/sec
   - 100% correctness (vs libart's 98.8%)
   - Inspired by libart's optimizations but cleaner implementation

3. **libart (C) is fast but not perfect**
   - 72.2 bytes overhead per key
   - 98.8% correctness on our dataset (possible edge case bugs)
   - Still excellent performance

4. **BTreeMap is a solid baseline**
   - 74.6 bytes overhead per key
   - High fanout (16-32 keys/node) minimizes node count
   - stdlib implementation is highly optimized

5. **Fixed-key-size ART implementations are disasters**
   - art, rart, blart all use fixed-size arrays
   - Forces 256 bytes per key regardless of actual length
   - Average URL is ~49 bytes, wasting 207 bytes per key

---

## FastArt: Our Best Mutable ART

Key optimizations inspired by libart (C):

1. **Pointer tagging** - Low bit distinguishes leaf vs internal node
   - Eliminates separate enum discriminant
   - No Box overhead for leaves

2. **Inline key storage** - Keys embedded directly after leaf header
   - Like C's flexible array members
   - No separate Vec allocation per key

3. **Compact node header** - 16 bytes, matching libart
   ```rust
   #[repr(C, packed)]
   pub struct NodeHeader {
       pub partial_len: u32,      // 4 bytes
       pub node_type: NodeType,   // 1 byte
       pub num_children: u8,      // 1 byte
       pub partial: [u8; 10],     // 10 bytes = MAX_PREFIX
   }
   ```

4. **Terminating byte** - Keys stored with null terminator
   - Handles prefix-of-another-key correctly
   - Same approach as libart

5. **Raw allocation** - Uses `std::alloc` directly
   - No Box/Vec overhead per node
   - Precise memory control

Node sizes:
- NodeHeader: 16 bytes
- Node4: 48 bytes (header + 4 keys + 4 pointers)
- Node16: 160 bytes (header + 16 keys + 16 pointers)
- Node48: 664 bytes (header + 256 index + 48 pointers)
- Node256: 2064 bytes (header + 256 pointers)
- Leaf: 12 bytes + key length (value + key_len + key bytes)

---

## Architecture

### FrozenLayer (Immutable) - Best Memory Efficiency

Built on the `fst` crate (Finite State Transducers):
- Shares prefixes AND suffixes (unlike ART which only shares prefixes)
- Provides compression rather than overhead
- O(n) lookup where n is key length
- Supports range queries and prefix scans
- Must be built from sorted keys

### FastArt (Mutable) - Best ART Implementation

Libart-inspired Adaptive Radix Tree:
- Pointer tagging for leaves vs internal nodes
- Inline key storage in leaves
- Compact 16-byte node headers
- Adaptive node types: Node4, Node16, Node48, Node256
- Raw memory allocation for minimal overhead

### ArenaArt (Mutable) - Alternative ART

Arena-based Adaptive Radix Tree:
- All nodes stored in `Vec<ArenaNode<V>>` (no per-node allocation overhead)
- Variable-length keys stored in separate data arena
- 4-byte `NodeRef` pointers instead of 8-byte raw pointers
- Slightly higher overhead but simpler memory management

---

## Recommendations

### For Read-Heavy/Frozen Data: Use FrozenLayer
```rust
let mut builder = FrozenLayerBuilder::new()?;
for (key, value) in sorted_data {
    builder.insert(key, value)?;
}
let frozen = builder.finish()?;
```

### For Mutable Data: Use FastArt
```rust
use memkv::FastArt;

let mut art = FastArt::new();
art.insert(b"key", 42);
assert_eq!(art.get(b"key"), Some(42));
```

### For Hybrid Workloads: FST Base + Mutable Delta
```rust
// Periodically freeze mutable layer and merge
let base = FrozenLayer::build(sorted_base)?;
let delta = FastArt::new();

// Lookup: check delta first, fall back to base
fn get(&self, key: &[u8]) -> Option<u64> {
    self.delta.get(key).or(self.base.get(key))
}
```

---

## Implementation Details

### Memory Measurement

Use jemalloc's allocation tracking for Rust allocations:
```rust
use tikv_jemalloc_ctl::{epoch, stats};

fn get_allocated() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}
```

Use RSS for raw allocations (FastArt, libart):
```rust
fn get_rss() -> usize {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
    let pages: usize = statm.split_whitespace().nth(1).unwrap().parse().unwrap();
    pages * 4096
}
```

### Tested External Crates

| Crate | Key Type | Verdict |
|-------|----------|---------|
| art-tree | Variable | Best external ART (but 164 bytes/key) |
| blart | Fixed [u8; 256] | Poor for strings |
| art | Fixed [u8; N] | Poor for strings |
| rart | Fixed ArrayKey | Worst of all! |

---

## Future Work

1. **Concurrent FastArt**: Lock-free or fine-grained locking
2. **SIMD Node16**: SSE2/AVX2 for child lookup
3. **Memory pooling**: Fixed-size allocators for nodes
4. **Persistence**: Memory-mapped FST for disk-backed storage
5. **Incremental FST updates**: Efficient load-update-write cycles

---

## Conclusion

For memory-efficient key-value storage of string keys:

| Use Case | Recommendation | Overhead/Key |
|----------|----------------|--------------|
| Immutable data | FrozenLayer (FST) | -16 bytes (compression!) |
| Mutable data | FastArt | 58 bytes |
| Simple/portable | BTreeMap | 75 bytes |

**FastArt achieves our goal of beating libart** while providing:
- 19% less memory overhead (58 vs 72 bytes/key)
- 100% correctness (vs libart's 98.8%)
- Faster inserts (5.1M vs 4.9M ops/sec)
- Pure Rust, no unsafe C dependencies

The original target of <10 bytes overhead per key is achievable with FST for immutable data. For mutable structures, ~60 bytes overhead is excellent and competitive with the best C implementations.
