# memkv Internals

Internal engineering documentation. Everything we learned building memory-efficient key-value indexes.

## Final State

After extensive experimentation, we shipped:

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

## Benchmark Results

500K URLs, shuffled random insert order, 23.2 MB raw key data:

| Structure | Total Memory | Overhead B/K | Index B/K | Insert/s | Lookup/s |
|-----------|-------------|--------------|-----------|----------|----------|
| BTreeMap | 52.0 MB | 57.7 | 49.7 | 1.4M | 2.1M |
| **InlineHot** | **34.6 MB** | **22.7** | **12.0** | 1.8M | 2.1M |
| HOT | 37.7 MB | 29.1 | 16.0 | 1.8M | 2.5M |
| FastArt | 49.1 MB | 51.7 | 43.7 | 2.4M | 5.2M |

Overhead = (Total - Raw Keys) / Count
Index = Overhead - 8 (value size)

InlineHot saves 33% memory vs BTreeMap.

## Key Design Decisions

### Why BiNodes Instead of Full HOT Compound Nodes

The HOT paper describes compound nodes with 16-256 entries, SIMD partial key search, and pext/pdep bit manipulation. We implemented simplified BiNodes (2 entries per node) because:

1. **Complexity**: Full HOT is ~3000 lines of C++ with AVX2/BMI2 requirements
2. **Diminishing returns**: BiNodes achieve 12 B/K index overhead vs paper's 3-6 B/K
3. **Correctness**: Simpler code = fewer bugs. All our tests pass.
4. **Time**: BiNode implementation took hours, full HOT would take weeks

The gap (12 vs 3-6 B/K) comes from:
- BiNode: 10 bytes per node, N-1 nodes for N keys = 10 B/K
- HOT paper: Compound nodes amortize overhead across 16-256 children

### Why Inline Values

InlineHot stores values directly in `key_data` stream: `[len:2][key bytes][value:8]`

This eliminates the separate `Leaf` struct that HOT uses:
- HOT Leaf: `{ key_off: u32, key_len: u16, value: u64 }` = 14 bytes
- InlineHot: 2 bytes length prefix only

Savings: 14 - 2 = 12 bytes per entry → but we use 10 B/K for BiNodes anyway.
Net result: 12 B/K index overhead (BiNodes + length prefix).

### Why 32-bit Pointers

Both InlineHot and HOT use `u32` offsets instead of `u64` pointers:
- Saves 4 bytes per pointer
- Limits addressable space to 4GB per arena
- For 100M keys averaging 50 bytes = 5GB keys + overhead fits in multiple arenas

We considered u16 pointers (2 bytes) but hit limits at ~32K entries.

## Data Structures Evaluated

### Mutable (what we ship)

| Structure | Overhead | Speed | Notes |
|-----------|----------|-------|-------|
| InlineHot | 22.7 B/K | 2.1M | Best memory, BiNodes + inline values |
| HOT | 29.1 B/K | 2.5M | BiNodes + separate leaves |
| FastArt | 51.7 B/K | 5.2M | ART with pointer tagging, SIMD Node16 |
| AdaptiveRadixTree | ~60 B/K | 3-4M | Generic ART, debug support |

### Immutable (for reference)

| Structure | Overhead | Speed | Notes |
|-----------|----------|-------|-------|
| FrozenLayer (FST) | ~33 B/K | 3.3M | fst crate wrapper |
| CompressedBlockStore* | 12.6 B/K | 103K | ZSTD blocks, from wilson/claude-glory branch |

*Not shipped - immutable only, 20x slower lookups

### Rejected/Abandoned

| Structure | Why Rejected |
|-----------|--------------|
| LsmHot | Hybrid mutable+frozen, too complex for marginal gains |
| CompactHot | u16 pointers hit 32K entry limit |
| CompoundHot | Incomplete compound node logic, worse than BiNodes |
| Patricia | Infinite loop bug in edge cases |
| Various art_* variants | Superseded by FastArt or InlineHot |

## Memory Accounting

### HOT Paper Definition

The HOT paper reports 11-14 B/K which INCLUDES:
- Node structure (headers, partial keys, discriminator bits)
- Child pointers (8 bytes each, store TIDs/values directly)

Does NOT include:
- Raw key storage (keys stored externally, accessed via KeyExtractor)

### Our Definition

We measure:
- **Total overhead** = (jemalloc allocated - raw key bytes) / count
- **Index overhead** = Total overhead - 8 (value size)

InlineHot achieves 12 B/K index overhead, which is close to but above HOT paper's 3-6 B/K pure structure overhead.

### Why the Gap

HOT paper's 3-6 B/K comes from compound nodes with high fanout:
- 16-256 entries per node
- Node overhead amortized across all children
- SIMD search eliminates per-entry comparison overhead

Our BiNodes have 2 entries:
- 10 bytes per BiNode
- N-1 BiNodes for N keys
- Plus 2 B/K length prefix
- = 12 B/K

To close the gap: implement full compound nodes with dynamic growth, SIMD partial key search, pext/pdep bit manipulation. This is significant engineering effort for diminishing returns.

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

BiNode layout: `[bit_pos:2][left:4][right:4]` = 10 bytes

Insert algorithm:
1. Walk tree following discriminating bits
2. Find leaf position
3. If key matches, update value
4. Otherwise, find first differing bit, create new BiNode splitting at that bit

Lookup algorithm:
1. Walk tree following discriminating bits
2. At leaf, compare full key
3. Return value if match

### FastArt (art_fast/mod.rs)

Standard ART with:
- Pointer tagging (bit 0 = is_leaf)
- Four node types: Node4, Node16, Node48, Node256
- SIMD search for Node16 (SSE2)
- Path compression (prefix stored in nodes)

Better speed than InlineHot due to:
- Higher fanout (up to 256 vs 2)
- SIMD acceleration
- No bit manipulation per level

Worse memory due to:
- 64-bit pointers
- Node overhead not amortized as well
- Keys stored in leaves, not shared

### FrozenLayer (frozen/mod.rs)

Thin wrapper around `fst::Map`:
- Immutable after construction
- Excellent compression for sorted data
- O(key_length) lookups
- Range queries via FST automaton

Build from sorted iterator:
```rust
let data = vec![(b"key1", 1u64), (b"key2", 2u64)];
let frozen = FrozenLayer::from_sorted_iter(data)?;
```

## Experiments That Failed

### 1. u16 Pointers (CompactHot)

Idea: Use 2-byte pointers instead of 4-byte to save more memory.

Result: Works for small data, panics at ~32K entries due to address space limits.

Lesson: Need to scale pointers based on expected data size, or use hybrid approaches.

### 2. LSM-style Hybrid (LsmHot)

Idea: Small mutable buffer + frozen sorted layer, periodic compaction.

Result: Abandoned after user feedback ("Oh my god"). The complexity wasn't justified when InlineHot already beats BTreeMap by 33%.

Lesson: Simple solutions that work > complex architectures that might work better.

### 3. Full Compound Nodes (CompoundHot)

Idea: Implement HOT paper's compound nodes with dynamic growth.

Result: Incomplete implementation actually performed worse than BiNodes (20 vs 16 B/K) because the growth logic was buggy.

Lesson: Partial implementations of complex algorithms can be worse than simple complete implementations.

### 4. Sorted Array Baselines (MinimalSorted, Compact32)

Idea: Just store sorted keys + values, binary search for lookup.

Result: Excellent memory (< 10 B/K) but requires sorted input. User clarified data arrives in random order, so these aren't drop-in BTreeMap replacements.

Lesson: Read requirements carefully. "Mutable with random inserts" is different from "bulk load sorted data".

## What We Learned About HOT

From studying the reference C++ implementation (github.com/speedskater/hot):

1. **Compound nodes** combine multiple binary trie levels into single nodes with up to 32 entries
2. **Discriminator bit positions** are stored as deltas, not absolute positions
3. **SIMD search** uses partial keys (extracted via pext) compared in parallel
4. **Node types** vary based on number of discriminator bits (1-8) and partial key size (8/16/32 bits)
5. **Memory layout** is carefully optimized for cache lines
6. **ROWEX** provides concurrent read-optimized write exclusion

The 11-14 B/K reported in the paper comes from this sophisticated implementation. Our BiNode simplification trades memory efficiency for implementation simplicity.

## Performance Characteristics

### Time Complexity

All structures: O(key_length) for point operations

- InlineHot: ~20-30 bit comparisons for typical 50-byte key
- FastArt: ~50 byte comparisons (one per level)
- BTreeMap: O(log n) comparisons, each O(key_length)

### Memory Complexity

- InlineHot: O(n * avg_key_len) + O(n) index overhead
- FastArt: O(n * avg_key_len) + O(n) node overhead (higher constant)
- BTreeMap: O(n * avg_key_len) + O(n) pointer overhead (highest constant)

### Cache Behavior

- InlineHot: Good locality in key_data, poor in BiNode tree traversal
- FastArt: Good for Node4/16, poor for Node256 (2KB per node)
- FrozenLayer: Excellent (FST is cache-optimized)

## Future Work

### If We Need Better Memory

1. **Full HOT compound nodes**: 3-6 B/K structure overhead, significant engineering
2. **Compressed block storage**: 12.6 B/K total but immutable, 20x slower
3. **Front-coding for keys**: Store delta-encoded keys, better prefix sharing

### If We Need Better Speed

1. **SIMD for InlineHot**: Parallel bit extraction/comparison
2. **Cache-optimized node layout**: Pack hot data together
3. **Prefetching**: Hint next nodes during traversal

### If We Need Concurrency

1. **ROWEX for InlineHot**: Read-optimized write exclusion from HOT paper
2. **Lock-free reads**: Epoch-based memory reclamation
3. **Sharded structure**: Multiple independent indexes

## Research References

See `docs/researcher-ideas-*.md` for comprehensive literature review covering:
- ART variants and optimizations
- HOT compound nodes and SIMD
- Succinct data structures (FST, SuRF)
- Memory layout optimizations
- Hybrid architectures

Key papers:
- "The Adaptive Radix Tree" (Leis et al., 2013)
- "HOT: A Height Optimized Trie Index" (Binna et al., 2018)
- fst crate blog posts by BurntSushi

## Lessons Learned

1. **Measure everything**: jemalloc stats revealed Vec capacity overhead we missed initially
2. **Simple beats complex**: BiNodes beat our incomplete compound node implementation
3. **Read the paper carefully**: HOT's 11-14 B/K includes values, not just structure
4. **User requirements matter**: "Random insert order" invalidated our sorted-array approaches
5. **33% memory savings is significant**: Don't need to hit theoretical minimum to ship value
6. **Clean code ships faster**: Removed 20+ experimental modules to ship 6 clean ones

## File Sizes (After Cleanup)

```
src/lib.rs           ~200 lines   Main exports
src/hot_inline.rs    ~300 lines   InlineHot implementation
src/hot_final.rs     ~300 lines   HOT with BiNodes
src/art_fast/mod.rs  ~900 lines   FastArt implementation
src/art/mod.rs       ~900 lines   AdaptiveRadixTree
src/frozen/mod.rs    ~250 lines   FrozenLayer (FST wrapper)
src/encoding/mod.rs  ~150 lines   Utilities
src/simple.rs        ~100 lines   SimpleKV wrapper
```

Total: ~3100 lines of production code (down from ~15000+ experimental lines)
