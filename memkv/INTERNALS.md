# memkv Internals

Everything we know about memory-efficient key-value indexes. This document synthesizes our implementation experience, research literature, design debates, measurements, and lessons learned.

---

## Table of Contents

1. [What We Shipped](#what-we-shipped)
2. [Benchmark Results](#benchmark-results)
3. [The Problem: Memory at Scale](#the-problem-memory-at-scale)
4. [Data Structure Landscape](#data-structure-landscape)
5. [Our Design Decisions](#our-design-decisions)
6. [Implementation Details](#implementation-details)
7. [Experiments That Failed](#experiments-that-failed)
8. [Research Deep Dive](#research-deep-dive)
9. [Memory Optimization Techniques](#memory-optimization-techniques)
10. [Future Directions](#future-directions)
11. [Lessons Learned](#lessons-learned)

---

## What We Shipped

```
src/
├── lib.rs          # Main exports: InlineHot, FastArt, HOT, FrozenLayer, MemKV
├── hot_inline.rs   # InlineHot - best memory efficiency (22.7 B/K overhead)
├── hot_final.rs    # HOT - BiNode variant (29.1 B/K overhead)
├── art_fast/       # FastArt - best speed (5.2M lookups/s)
├── art/            # AdaptiveRadixTree - generic ART with debug support
├── frozen/         # FrozenLayer - FST-based immutable storage
├── encoding/       # Varint, front-coding utilities
└── simple.rs       # SimpleKV - basic thread-safe wrapper
```

~3100 lines of production code, down from 15000+ experimental lines.

---

## Benchmark Results

500K URLs, shuffled random insert order, 23.2 MB raw key data:

| Structure | Total Memory | Overhead B/K | Index B/K | Insert/s | Lookup/s |
|-----------|-------------|--------------|-----------|----------|----------|
| BTreeMap | 52.0 MB | 57.7 | 49.7 | 1.4M | 2.1M |
| **InlineHot** | **34.6 MB** | **22.7** | **12.0** | 1.8M | 2.1M |
| HOT | 37.7 MB | 29.1 | 16.0 | 1.8M | 2.5M |
| FastArt | 49.1 MB | 51.7 | 43.7 | 2.4M | 5.2M |

**Definitions:**
- Overhead = (Total Memory - Raw Key Bytes) / Key Count
- Index = Overhead - 8 (the u64 value size)

**Result: InlineHot saves 33% memory vs BTreeMap.**

---

## The Problem: Memory at Scale

### The Tyranny of the 64-bit Pointer

At billion-key scale, structural overhead dominates. A naive binary tree node:

```rust
struct Node {
    left: *mut Node,   // 8 bytes
    right: *mut Node,  // 8 bytes
    key: String,       // 24 bytes (ptr + len + cap) + heap
    value: u64,        // 8 bytes
}
```

The two pointers alone consume 16 bytes. With allocator headers (8-16 bytes for chunk metadata), total overhead reaches 32 bytes per node. For 1 billion keys: **32 GB of RAM just for tree topology**, before storing a single byte of actual data.

Rust's `BTreeMap` mitigates this by storing multiple keys per node (higher fanout), but still requires:
- 64-bit pointers between nodes
- Per-key heap allocations for strings
- Internal fragmentation in partially-filled nodes

### Allocator Overhead and Fragmentation

Standard allocators (malloc, jemalloc) optimize for general-purpose patterns, not billions of tiny objects.

When allocating a `String` in Rust:
- Stack: 24 bytes (pointer, capacity, length)
- Heap: string data + allocator metadata (~8 bytes)
- Fragmentation: 10-20% lost to alignment padding

For maximum density, we must use **arena allocation**: request large memory blocks from the OS and pack objects sequentially, reducing per-object overhead to near zero.

### The Comparison Bottleneck

Real-world keys (URLs, file paths) exhibit:
- Variable length (few bytes to hundreds)
- High prefix redundancy ("https://www." shared across millions)

In comparison-based structures (B-Tree, Skip List), searching requires O(log N) comparisons, each O(k) for key length k. Crucially, B-Trees store keys explicitly—`system/config/network` and `system/config/display` both store the full prefix `system/config/`.

This directs us toward **tries**, where common prefixes are stored exactly once.

### Throughput Floor

Target: 100K ops/sec. This is modest (modern structures hit millions), so we can trade CPU cycles for memory—aggressive compression, complex bit manipulation—as long as we exceed this floor. This "CPU-for-RAM" trade-off guides all decisions.

---

## Data Structure Landscape

### Comparison of Approaches

| Structure | Bytes/Key | Range Queries | Updates | Notes |
|-----------|-----------|---------------|---------|-------|
| **HOT** | 11-14 | Excellent | Yes | Best for strings, needs SIMD |
| **ART** | 15-30 (52 worst) | Good | Yes | Practical, multiple crates |
| **FST** | ~1.25 bits/node | Excellent | No | Static only |
| **Judy arrays** | 5-7 | Yes | Yes | Complex, partial Rust support |
| **B+ tree** | 20-40 | Excellent | Yes | Proven, prefix compression helps |
| **patricia_tree** | ~32 | Prefix only | Yes | Production Rust crate |
| **SILT trie** | 0.4 | Limited | No | Entropy-coded, read-only |

### Adaptive Radix Tree (ART)

The 2013 paper by Leis et al. introduced adaptive node sizes to solve the trie fanout problem:

- **Node4**: 4 keys + 4 pointers (smallest)
- **Node16**: SIMD-searchable (SSE2/NEON), 16 children
- **Node48**: 256-entry index → 48 children, O(1) lookup
- **Node256**: Direct indexing, 256 pointers

ART guarantees ≤52 bytes/key worst case, typically much lower. Used by DuckDB, HyPer.

**Limitations for our use case:**
- Fixed 8-bit (byte) spans—sparse keys waste nodes
- 64-bit pointers still dominate overhead
- Path compression helps but doesn't eliminate the span issue

### Height Optimized Trie (HOT)

The 2018 SIGMOD paper addresses ART's byte-alignment limitation:

**Key innovation: Dynamic span.** A HOT node branches on arbitrary bit positions (e.g., bits 3, 4, 12, 15 simultaneously), not fixed byte boundaries. Multiple binary Patricia trie levels are combined into "compound nodes" with target fanout K (typically 32).

**Result:** Consistently high fanout regardless of key distribution. Tree height = log₃₂(N), much shallower than binary tries or byte-aligned ART.

**Memory:** 11-14 bytes/key for string workloads—roughly half of ART.

**Implementation complexity:**
- PEXT/PDEP instructions (BMI2) for bit extraction
- SIMD partial key search (AVX2)
- 9 physical node layouts
- ~3000 lines of C++ in reference implementation

### Succinct Structures (FST, SuRF)

FST (Fast Succinct Tries) achieves ~10 bits per node using LOUDS encoding (Level-Order Unary Degree Sequence). Navigation via rank/select bit operations.

**Trade-off:** Mutability. Inserting a bit into the middle requires shifting all subsequent bits—O(N). Practical use requires LSM-tree architecture (immutable runs + merge).

SuRF extends FST for approximate range queries at 10-14 bits/key, but stores truncated keys (false positives acceptable for filters, not primary storage).

### Learned Indexes

ALEX, LIPP excel at integer keys using piecewise linear regression to predict positions. For strings, this breaks down: keys sharing long prefixes map to the same integer when truncated.

LITS (Learned Index with Hash-enhanced Prefix Table) addresses this with:
- HPT: Hash table skipping dense top trie levels
- PMSS: Dynamic choice between learned model and trie per micro-distribution

Complexity is extreme (training, retraining, overflow arrays). For deterministic guarantees, HOT is preferable.

---

## Our Design Decisions

### Why BiNodes Instead of Full HOT Compound Nodes

The HOT paper describes compound nodes with 16-256 entries, SIMD partial key search, pext/pdep. We implemented simplified BiNodes (2 entries per node):

1. **Complexity**: Full HOT is ~3000 lines of C++ with AVX2/BMI2 requirements
2. **Diminishing returns**: BiNodes achieve 12 B/K index overhead vs paper's 3-6 B/K
3. **Correctness**: Simpler code = fewer bugs. All tests pass.
4. **Time**: BiNode implementation took hours, full HOT would take weeks

The gap (12 vs 3-6 B/K) comes from fanout:
- BiNode: 10 bytes/node, N-1 nodes for N keys = 10 B/K
- HOT: Compound nodes amortize overhead across 16-256 children

### Why Inline Values

InlineHot stores values directly in `key_data`: `[len:2][key bytes][value:8]`

This eliminates the separate Leaf struct:
- HOT Leaf: `{ key_off: u32, key_len: u16, value: u64 }` = 14 bytes
- InlineHot: 2 bytes length prefix only

Net result: 12 B/K index overhead (10 BiNodes + 2 length prefix).

### Why 32-bit Pointers

Both InlineHot and HOT use `u32` offsets instead of `u64` pointers:
- Saves 4 bytes per pointer (50% reduction)
- Limits addressable space to 4GB per arena
- For 100M keys averaging 50 bytes: fits comfortably

We tried u16 pointers (2 bytes) but hit limits at ~32K entries.

### Why Not Hybrid Delta+Frozen

We considered LSM-style architecture (small mutable ART + frozen FST, periodic merge). Abandoned because:
- InlineHot already beats BTreeMap by 33%
- Complexity wasn't justified for marginal gains
- User feedback: "Oh my god" (not positive)

Simple solutions that work > complex architectures that might work better.

---

## Implementation Details

### InlineHot (hot_inline.rs)

```rust
pub struct InlineHot {
    key_data: Vec<u8>,   // [len:2][key][value:8]... contiguous
    nodes: Vec<u8>,      // BiNodes packed contiguously
    root: Ptr,           // 32-bit tagged pointer
    count: usize,
}
```

**BiNode layout:** `[bit_pos:2][left:4][right:4]` = 10 bytes

**Ptr encoding (32-bit tagged pointer):**
- Bit 31 = 1: leaf pointer (bits 0-30 = offset into key_data)
- Bit 31 = 0: node pointer (bits 0-30 = offset into nodes)
- 0xFFFFFFFF: null

**Insert algorithm:**
1. Walk tree following discriminating bits
2. Find leaf position
3. If key matches, update value in-place
4. Otherwise, find first differing bit, create new BiNode splitting at that bit

**Lookup algorithm:**
1. Walk tree following discriminating bits
2. At leaf, compare full key
3. Return value if match

### FastArt (art_fast/mod.rs)

Standard ART implementation inspired by libart (C):

- **Pointer tagging**: Low bit distinguishes leaf vs internal node
- **Four node types**: Node4, Node16, Node48, Node256
- **SIMD search**: SSE2 for Node16 child lookup
- **Path compression**: Prefix stored in nodes, skip matching bytes

**Why faster than InlineHot:**
- Higher fanout (up to 256 vs 2)
- SIMD acceleration for Node16
- No per-level bit manipulation

**Why more memory:**
- 64-bit pointers
- Node overhead not amortized as well
- Keys stored in leaves, not shared

### FrozenLayer (frozen/mod.rs)

Thin wrapper around `fst::Map`:

```rust
let data = vec![(b"key1", 1u64), (b"key2", 2u64)];
let frozen = FrozenLayer::from_sorted_iter(data)?;
```

- Immutable after construction
- Excellent compression for sorted data
- O(key_length) lookups
- Range queries via FST automaton

---

## Experiments That Failed

### 1. u16 Pointers (CompactHot)

**Idea:** Use 2-byte pointers instead of 4-byte.

**Result:** Works for small data, panics at ~32K entries due to address space limits (65K max offsets, but node allocation is sparse).

**Lesson:** Pointer size must scale with expected data size.

### 2. LSM-style Hybrid (LsmHot)

**Idea:** Small mutable buffer + frozen sorted layer, periodic compaction.

**Result:** Abandoned. Complexity wasn't worth it when InlineHot already ships 33% memory savings.

**Lesson:** Don't over-engineer. Ship value incrementally.

### 3. Full Compound Nodes (CompoundHot)

**Idea:** Implement HOT paper's compound nodes with dynamic growth (2→16→256 entries).

**Result:** Incomplete implementation performed worse than BiNodes (20 vs 16 B/K) due to buggy growth logic.

**Lesson:** Partial implementations of complex algorithms can be worse than simple complete ones.

### 4. Sorted Array Baselines (MinimalSorted, Compact32)

**Idea:** Store sorted keys + values, binary search for lookup.

**Result:** Excellent memory (<10 B/K) but requires sorted input. User clarified data arrives in random order.

**Lesson:** Read requirements carefully. "Mutable with random inserts" ≠ "bulk load sorted data".

### 5. Patricia Trie

**Idea:** Classic Patricia trie implementation.

**Result:** Infinite loop bug in edge cases. Disabled.

**Lesson:** Edge cases in trie implementations are subtle. Test extensively.

---

## Research Deep Dive

### ART Variants and Research Directions

**Original ART (2013)** - Leis, Kemper, Neumann at TU Munich:
- Four adaptive node sizes
- Path compression, lazy expansion
- ≤52 bytes/key worst case

**Concurrency: Optimistic Lock Coupling & ROWEX (2016)**:
- OLC and ROWEX scale well, easier than lock-free
- Epoch-based memory reclamation for safe deferred frees
- Wait-free readers with version numbers

**START (2020)**: Nodes spanning multiple keybytes, 85% faster reads, 45% faster mixed workloads.

**Persistent Memory Variants:**
- WORT/WOART (FAST 2017): Single 8-byte atomic write per update
- P-ART/RECIPE (SOSP 2019): ROWEX for persistent memory
- PFtree (DASFAA 2023): Optimized for eADR platforms

**Disaggregated Memory:**
- SMART (OSDI 2023): First radix tree for DM, 6.1x higher write throughput vs B+ trees

**Notable Implementations:**

| Implementation | Language | Features |
|----------------|----------|----------|
| DuckDB | C++ | Swizzlable pointers for disk persistence |
| libart | C | Simple single-threaded reference |
| ARTSynchronized | C++ | OLC and ROWEX synchronization |
| Congee | Rust | ART-OLC, SIMD, fixed 8-byte keys, 150M ops/s on 32 cores |
| art-rs | Rust | Prefix-caching via hash table |

### HOT Deep Dive

**Core mechanics:**

1. **Dynamic span**: Branch on arbitrary bit positions, not byte boundaries
2. **Compound nodes**: Combine multiple Patricia trie levels into one node (fanout up to 32)
3. **Bit extraction**: PEXT instruction extracts discriminative bits into contiguous integer
4. **SIMD search**: Compare extracted bits against child discriminants in parallel

**Node layout (conceptual):**
```
Header (1 byte): node type, entry count
Discriminative mask (u64): bit positions that differentiate children
Partial keys (array): PEXT results for each child
Children (array of pointers): child node/leaf pointers
```

**Search with PEXT:**
```rust
// Extract discriminative bits from search key
let compressed = _pext_u64(key_bits, node.mask);
// SIMD compare against all partial keys
let matches = _mm256_cmpeq_epi16(compressed, partial_keys);
// Find matching child index
let idx = _mm256_movemask_epi8(matches).trailing_zeros();
```

**Memory accounting:**
- HOT paper reports 11-14 B/K which INCLUDES 8-byte child pointers (storing TIDs/values)
- Pure structure overhead: 3-6 B/K
- Does NOT include raw key storage (keys external, accessed via KeyExtractor)

### Succinct Data Structures

**LOUDS encoding**: Represent tree structure in two bit-vectors:
- D-Labels: Degree sequence
- Tree structure: Navigation bits

Navigation via rank (count 1-bits up to position) and select (find nth 1-bit). Can be computed in O(1) with precomputed tables.

**SuRF**: Succinct Range Filter
- ~10 bits per node
- Stores truncated keys (minimum distinguishing prefix)
- False positives acceptable for filters
- Used in RocksDB for prefix bloom filters

### Order-Preserving Compression

**HOPE (High-speed Order-Preserving Encoder):**
- Dictionary-based compression preserving lexicographic order
- Frequent patterns ("http://", ".com") get short codes
- Hu-Tucker algorithm: Optimal prefix-free codes with ordered leaves
- 30-50% key size reduction

**Front Coding:**
```
system/log/a           → full key
system/log/b           → (11, 'b')  // 11 bytes shared prefix
system/log/c           → (11, 'c')
```
25-80% compression for hierarchical keys.

**FSST (Fast Static Symbol Table):**
- Byte-aligned codes (1 byte → up to 8 bytes)
- Fast decompression via SIMD gather/scatter
- Good for value storage or cold key storage

---

## Memory Optimization Techniques

### Pointer Compression

Replace 64-bit pointers with 32-bit offsets:

```rust
fn compress(ptr: *const u8, base: *const u8) -> u32 {
    ((ptr as usize - base as usize) >> 3) as u32  // 8-byte alignment
}
fn decompress(offset: u32, base: *const u8) -> *const u8 {
    unsafe { base.add((offset as usize) << 3) }
}
```

With 8-byte alignment, 32 bits address 32GB. V8's implementation achieves 40% memory reduction for pointer-heavy structures.

### Pointer Swizzling

Bottom bits of aligned pointers are always zero. Use them for metadata:
- Bit 0: is_leaf flag
- Bit 1: lock bit (for spinlocks)
- Bit 2: node_type discriminant

Eliminates separate enum tags, saving 1-8 bytes per pointer (due to padding).

### Arena Allocation

Allocate large blocks (1GB), pack objects sequentially:
- Eliminates per-allocation overhead (8-16 bytes/malloc)
- Improves cache locality
- Enables bulk deallocation

Use `bumpalo` for phase-oriented allocation, `typed-arena` for single-type arenas with destructors.

### Varint Encoding

Variable-length integers for lengths and small values:
- LEB128: 1-10 bytes for u64, continuation bits per byte
- GroupVarint: Batch 4 integers, length prefix
- Saves 60%+ for typical metadata

---

## Future Directions

### If We Need Better Memory

1. **Full HOT compound nodes**: 3-6 B/K structure, significant engineering (weeks)
2. **HOPE key compression**: 30-50% key size reduction, order-preserving
3. **Compressed block storage**: 12.6 B/K total, immutable, 20x slower (see wilson/claude-glory branch)
4. **Front-coding for keys**: Delta-encoded keys in sorted blocks

### If We Need Better Speed

1. **SIMD for InlineHot**: Parallel bit extraction, PEXT-based lookup
2. **Cache-optimized layout**: Pack hot data together, prefetch hints
3. **HPT (Hash Prefix Table)**: Skip top trie levels via hash table

### If We Need Concurrency

1. **ROWEX**: Read-Optimized Write EXclusion from HOT paper
2. **Lock-free reads**: Version numbers + epoch-based reclamation
3. **Sharding**: Multiple independent indexes, partition by key prefix

### Research Worth Monitoring

- **LITS (2024)**: Hash-enhanced prefix table + learned models for strings
- **Memento Filter (SIGMOD 2025)**: Dynamic range filter with inserts/deletes
- **PGM-index**: 83-1140x less space for sorted integers (mapping overhead negates gains for strings)
- **CXL/heterogeneous memory**: New memory tiers change the optimization landscape

---

## Lessons Learned

### Technical

1. **Measure with jemalloc**: Vec capacity overhead was invisible until we used `tikv_jemalloc_ctl::stats::allocated`
2. **Simple beats incomplete complex**: BiNodes outperformed our buggy compound node implementation
3. **Read papers carefully**: HOT's 11-14 B/K includes values (8 bytes), leaving only 3-6 B/K for structure
4. **Pointer size matters**: u16 pointers seemed clever until we hit 32K entry limit

### Process

5. **Requirements first**: "Random insert order" invalidated all our sorted-array work
6. **Ship incrementally**: 33% memory savings is valuable even if not theoretical minimum
7. **Cut scope aggressively**: Removed 20+ experimental modules to ship 6 clean ones
8. **User feedback is signal**: "Oh my god" meant stop and reconsider

### Architecture

9. **Hybrid isn't always better**: LSM-style delta+frozen added complexity without proportional gains
10. **Fanout is king**: Higher fanout = fewer nodes = less overhead. BiNodes (fanout 2) are worst case.
11. **Inline storage wins**: Eliminating separate Leaf struct saved more than any algorithmic improvement

---

## Rust Ecosystem Notes

**Production-ready crates:**

| Crate | Memory | Range Queries | Notes |
|-------|--------|---------------|-------|
| `fst` | Exceptional | Full | Static only, BurntSushi |
| `patricia_tree` | 56-80% vs HashMap | Prefix-based | 885 dependents |
| `im` (OrdMap) | Good | Full | Structural sharing |
| `rart` | Good | Full | SIMD, versioned |

**Gaps we encountered:**
- No production HOT implementation in Rust
- Variable-length keys with both range queries AND memory efficiency
- SIMD-optimized pure Rust ART (existing crates don't fully exploit SIMD)
- Inline value storage (most tries store values separately)

---

## Code Reference

### InlineHot Core Types

```rust
// 32-bit tagged pointer
struct Ptr(u32);
const LEAF_BIT: u32 = 1 << 31;

impl Ptr {
    fn is_leaf(self) -> bool { self.0 & LEAF_BIT != 0 }
    fn leaf_off(self) -> u32 { self.0 & !LEAF_BIT }
    fn node_off(self) -> u32 { self.0 }
}

// Main structure
pub struct InlineHot {
    key_data: Vec<u8>,  // [len:2][key][value:8]...
    nodes: Vec<u8>,     // [bit_pos:2][left:4][right:4]...
    root: Ptr,
    count: usize,
}
```

### HOT Paper's Relative Pointer (Conceptual)

```rust
#[derive(Clone, Copy)]
struct RelPtr(u32);

impl RelPtr {
    fn new(index: u32, is_leaf: bool) -> Self {
        RelPtr((index << 1) | (is_leaf as u32))
    }
    fn index(&self) -> usize { (self.0 >> 1) as usize }
    fn is_leaf(&self) -> bool { (self.0 & 1) != 0 }
}
```

### HOT Paper's Node Search (Conceptual)

```rust
unsafe fn search_node(header: &NodeHeader, key_bits: u64) -> Option<RelPtr> {
    // Extract discriminative bits
    let compressed = core::arch::x86_64::_pext_u64(key_bits, header.mask);
    
    // SIMD lookup in partial key array
    // ... _mm256_cmpeq_epi16, _mm256_movemask_epi8 ...
    
    // Return child pointer
}
```

---

## File Inventory

```
src/lib.rs           ~200 lines   Exports, MemKV wrapper
src/hot_inline.rs    ~300 lines   InlineHot (best memory)
src/hot_final.rs     ~300 lines   HOT with BiNodes
src/art_fast/mod.rs  ~900 lines   FastArt (best speed)
src/art/mod.rs       ~900 lines   AdaptiveRadixTree (generic)
src/frozen/mod.rs    ~250 lines   FrozenLayer (FST wrapper)
src/encoding/mod.rs  ~150 lines   Varint utilities
src/simple.rs        ~100 lines   SimpleKV wrapper
```

---

*Last updated after shipping InlineHot with 33% memory savings vs BTreeMap.*
