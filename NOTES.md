# hot-rs

Development notes for a memory-efficient ordered map in Rust.

## Goal

Beat `BTreeMap<Vec<u8>, V>` memory usage by 30%+ on string keys while maintaining >100K ops/sec.

## Result

`HotTree<V>`: 33% less memory than BTreeMap on 500K URL keys.

```
Structure                  Memory    Overhead/Key
BTreeMap<Vec<u8>, u64>     52.0 MB   57.7 bytes
HotTree<u64>               34.6 MB   22.7 bytes
```

Final implementation: ~900 lines in `src/lib.rs`.

---

## Background

### Memory at scale

A binary tree node has 16 bytes just in child pointers, plus 8-16 bytes allocator overhead per node. At 1B keys that's 32GB for topology alone.

BTreeMap improves this with high fanout (16-32 keys/node) but still needs 64-bit pointers between nodes and per-key heap allocations.

### Why tries

String keys have high prefix redundancy. URLs sharing `https://www.` shouldn't store it millions of times. Tries store common prefixes once.

### CPU-for-RAM trade

100K ops/sec is modest. We could afford expensive bit manipulation if it saved memory.

---

## Structures considered

| Structure | Bytes/Key | Mutable | Notes |
|-----------|-----------|---------|-------|
| HOT | 11-14 | Yes | Bit-level branching, SIMD, complex |
| ART | 15-30 | Yes | Byte-aligned, used by DuckDB |
| FST | ~1.25 bits/node | No | LOUDS encoding, immutable |
| Judy | 5-7 | Yes | No good Rust impl |
| patricia_tree | ~32 | Yes | Production crate |

ART (2013, Leis et al.): Adaptive node sizes (Node4/16/48/256). Guarantees ≤52 bytes/key. Limitation: byte-aligned spans waste nodes on sparse keys.

HOT (2018 SIGMOD): Branches on arbitrary bit positions, not bytes. Compound nodes combine multiple trie levels. 11-14 bytes/key. Implementation requires PEXT/PDEP, AVX2, ~3000 lines C++.

FST: LOUDS encoding, ~10 bits/node. But inserting requires shifting all subsequent bits—O(N). Only practical for immutable data.

---

## Failed experiments

### ART implementation

Built full ART with Node4/16/48/256, path compression, SIMD Node16 search.

Result: ~52 bytes/key. 64-bit pointers and per-allocation overhead dominated. Many bugs in node transitions.

### u16 pointers

2-byte pointers instead of 8.

Result: Panicked at ~32K entries. Address space exhausted.

### u32 pointers

4-byte pointers, 4GB max arena.

Result: Worked until 282M URL test (16GB raw data). Panicked.

Settled on 48-bit (6-byte) pointers. Addresses 128TB, only 2 bytes more than u32.

### LSM hybrid

Small mutable buffer + frozen FST layer, periodic compaction.

Result: Abandoned. Massive complexity for marginal gains over simpler approach.

### Full HOT compound nodes

Implemented compound nodes with dynamic growth (2→16→256 entries).

Result: Buggy growth logic performed worse than simple BiNodes (20 vs 16 B/K). Partial implementations of complex algorithms can underperform simple complete ones.

### Sorted array

Store sorted keys + values, binary search.

Result: Excellent memory (<10 B/K) but requires sorted input. Our use case has random insert order.

### Patricia trie

Result: Infinite loop bug in edge cases.

### OptimizedART (suffix-only leaves)

Store only key suffix in leaves.

Result: After inserting "sighthound", all "s"-prefixed keys returned None.

### SmallVec for prefixes

`SmallVec<[u8; 16]>` for inline prefix storage.

Result: Increased memory on URL dataset (most prefixes >16 bytes).

### FrozenLayer (FST wrapper)

Wrapper around `fst::Map`.

Result: 2.4x compression but immutable. Considered hybrid mutable+frozen but complexity wasn't justified.

---

## What worked

### BiNodes

HOT paper uses compound nodes with 16-256 entries. We simplified to binary (2 entries):

- Full HOT: ~3000 lines C++ with AVX2/BMI2
- BiNodes: ~300 lines Rust

Gap is 12 B/K (ours) vs 3-6 B/K (paper). BiNodes are worst-case fanout but still beat BTreeMap.

### Inline values

Original: values in separate Leaf struct (14 bytes each).

New: `key_data = [len:2][key bytes][value_index:2]...`

Eliminated Leaf struct. Saved ~6 bytes per entry.

### 48-bit pointers

Layout: bit 47 = leaf flag, bits 0-46 = offset.

Stored as 6 bytes in BiNodes. Addresses 128TB.

### Final structure

```rust
pub struct HotTree<V> {
    key_data: Vec<u8>,      // [len:2][key][value_idx:2]...
    values: Vec<Option<V>>,
    nodes: Vec<u8>,         // BiNodes: [bit_pos:2][left:6][right:6]
    root: Ptr,              // 48-bit tagged pointer
    count: usize,
}
```

BiNode: 14 bytes. `bit_pos` (2) + `left` (6) + `right` (6).

---

## Techniques used

Arena allocation: Pack objects in `Vec<u8>`. Eliminates 8-16 bytes/malloc overhead.

Pointer tagging: Bit 47 distinguishes leaf vs node. No separate enum tag.

Value indirection: Values in `Vec<Option<V>>`, referenced by index. Allows generic `V` without affecting key arena layout.

---

## Techniques not used

Full HOT compound nodes: 3-6 B/K structure overhead. Requires PEXT/PDEP, AVX2.

HOPE key compression: Order-preserving dictionary compression. 30-50% key reduction.

Front-coding: Delta-encode keys in sorted blocks. 25-80% compression. Requires sorted data.

SIMD for BiNodes: Would speed traversal but adds complexity.

---

## References

Papers:
- ART (2013): "The Adaptive Radix Tree" - Leis, Kemper, Neumann
- HOT (2018): "HOT: A Height Optimized Trie Index" - SIGMOD
- FST: Fast Succinct Tries, LOUDS encoding
- SuRF: Succinct Range Filters (RocksDB)
- HOPE: High-speed Order-Preserving Encoder

Rust crates evaluated:
- `fst`: Excellent memory, immutable only
- `patricia_tree`: 56-80% vs HashMap, prefix queries only
- `art-tree`: ~164 bytes/key
- `rart`: SIMD, versioned

Gap: No production HOT in Rust. No mutable trie with both memory efficiency and range queries.

---

## Lessons

- Vec capacity overhead invisible without jemalloc tracking
- HOT paper's "11-14 B/K" includes 8-byte values; structure overhead is 3-6 B/K
- u16 pointers hit limits at 32K entries
- Higher fanout = fewer nodes = less overhead; BiNodes are worst case
- Eliminating Leaf struct saved more than algorithmic improvements