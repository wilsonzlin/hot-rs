# memkv: Memory-Efficient Key-Value Storage Design Document

## Overview

This library provides memory-efficient storage for string keys with arbitrary values, designed to support billions of keys in memory with minimal overhead.

---

## Comprehensive Benchmark Results (December 2024)

### Dataset: 9.5 Million Real-World URLs (467 MB raw data)

| Implementation | Memory | Overhead/Key | Insert ops/s | Notes |
|---------------|--------|--------------|--------------|-------|
| **FrozenLayer (FST)** | **320 MB** | **-30.1 bytes** | 680K | **COMPRESSION!** |
| std::BTreeMap | 1,145 MB | 74.6 bytes | 3.8M | Best mutable |
| **memkv::ArenaArt** | **1,449 MB** | **108.1 bytes** | 3.3M | **Best ART** |
| art-tree (crate) | 1,961 MB | 164.4 bytes | 3.8M | Variable keys |
| blart (crate) | 3,286 MB | 310.3 bytes | 3.0M | Fixed 256B keys |
| art (crate) | 6,953 MB | 713.9 bytes | 1.0M | Fixed 256B keys |
| rart (crate) | 9,875 MB | 1,035.6 bytes | 1.6M | SIMD, fixed keys |

### Key Findings

1. **FST achieves NEGATIVE overhead** (-30 bytes/key = compression)
   - 2.4x compression ratio vs raw data
   - 194 MB FST for 467 MB of keys
   - Best for immutable/frozen data

2. **BTreeMap is surprisingly the best mutable structure**
   - 74.6 bytes overhead per key
   - High fanout (16-32 keys/node) minimizes node count
   - stdlib implementation is highly optimized

3. **memkv::ArenaArt beats ALL existing Rust ART crates**
   - 108 bytes overhead (1.5x BTreeMap, but ART semantics)
   - Fastest ART lookup (8.7M ops/sec)
   - Variable-length keys stored efficiently

4. **rart (SIMD-optimized) is actually the WORST!**
   - 1,035 bytes overhead per key (!!)
   - Fixed 256-byte keys waste massive memory
   - SIMD doesn't help when memory is the bottleneck

5. **Fixed-key-size ART implementations are disasters**
   - art, rart, blart all use fixed-size arrays
   - Forces 256 bytes per key regardless of actual length
   - Average URL is ~49 bytes, wasting 207 bytes per key

---

## Architecture

### FrozenLayer (Immutable) - Best Memory Efficiency

Built on the `fst` crate (Finite State Transducers):
- Shares prefixes AND suffixes (unlike ART which only shares prefixes)
- Provides compression rather than overhead
- O(n) lookup where n is key length
- Supports range queries and prefix scans
- Must be built from sorted keys

### ArenaArt (Mutable) - Best ART Implementation

Arena-based Adaptive Radix Tree:
- All nodes stored in `Vec<ArenaNode<V>>` (no per-node allocation overhead)
- Variable-length keys stored in separate data arena
- 4-byte `NodeRef` pointers instead of 8-byte `Box` pointers
- Adaptive node types: Node4, Node16, Node48, Node256

Node sizes (after boxing large arrays):
- Leaf: 16 bytes
- Node4: 48 bytes  
- Node16: 56 bytes (children array boxed)
- Node48: 56 bytes (arrays boxed)
- Node256: 56 bytes (children array boxed)

### BTreeMap (Standard Library)

Why it performs so well:
- B-tree node holds 11-21 keys per node (high fanout)
- Fewer total nodes than ART
- Cache-friendly sequential node layout
- Decades of optimization

---

## Why ART Struggles for Variable-Length Strings

The fundamental issue: **node proliferation**

For 9.5M URLs:
- ArenaArt creates ~14.4M nodes (1.5 nodes per key)
- Each node adds overhead even with arena allocation
- BTreeMap creates ~0.5-0.7M nodes (0.05-0.07 nodes per key)

ART excels when:
- Fixed-length keys (integers, hashes)
- High prefix sharing potential
- Concurrent access requirements

ART struggles when:
- Variable-length strings
- Low prefix sharing (random or hashed keys)
- Memory is the primary constraint

---

## Recommendations

### For Read-Heavy/Frozen Data: Use FrozenLayer
```rust
let mut builder = FrozenLayerBuilder::new();
for (key, value) in sorted_data {
    builder.insert(key, value)?;
}
let frozen = builder.finish()?;
```

### For Mutable Data: Use BTreeMap (or ArenaArt for ART semantics)
```rust
// BTreeMap is usually better!
let mut map: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
map.insert(key.to_vec(), value);

// ArenaArt if you need ART-specific features
let mut art: ArenaArt<u64> = ArenaArt::new();
art.insert(key, value);
```

### For Hybrid Workloads: FST Base + Mutable Delta
```rust
// Periodically freeze mutable layer and merge
let base = FrozenLayer::build(sorted_base)?;
let delta = BTreeMap::new();

// Lookup: check delta first, fall back to base
fn get(&self, key: &[u8]) -> Option<u64> {
    self.delta.get(key).or_else(|| self.base.get(key))
}
```

---

## Implementation Details

### Memory Measurement

Use jemalloc's allocation tracking for accurate measurements:
```rust
use tikv_jemalloc_ctl::{epoch, stats};

fn get_allocated() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}
```

RSS-based measurements are unreliable due to allocator memory reuse.

### Per-Allocation Overhead

jemalloc adds ~48 bytes overhead per allocation:
- UltraCompactArt: 1.46M Box allocations × 48 bytes = 70 MB wasted
- ArenaArt: Uses Vec storage, amortized overhead

### Tested External Crates

| Crate | Key Type | Verdict |
|-------|----------|---------|
| art-tree | Variable | Best external ART |
| blart | Fixed [u8; 256] | Poor for strings |
| art | Fixed [u8; N] | Poor for strings |
| rart | Fixed ArrayKey | Worst of all! |

---

## Future Work

1. **Hybrid store**: FST for frozen base + mutable delta layer
2. **Incremental FST updates**: Efficient load-update-write cycles
3. **Multi-FST approach**: Tiered compaction like LSM trees
4. **SIMD optimizations**: For Node16 child lookup
5. **Concurrent writes**: Epoch-based reclamation

---

## Conclusion

For memory-efficient key-value storage of string keys:

1. **Immutable data**: FST provides actual compression (-30 bytes/key)
2. **Mutable data**: BTreeMap (74 bytes/key) beats all ART implementations
3. **Our ArenaArt** (108 bytes/key) beats all existing Rust ART crates

The target of <10 bytes overhead per key is achievable with FST for immutable data, but remains challenging for mutable structures due to the inherent overhead of maintaining tree structures.
