# MemKV Development Scratchpad

> This is a living document for tracking progress, experiments, ideas, and issues.
> Updated continuously throughout development.

---

## Current Status

**Phase**: Phase 3 - Accurate Memory Measurement & Further Optimization
**Date Started**: 2024-12-05
**Last Updated**: 2024-12-05

### Completed
- [x] Create DESIGN.md reference document
- [x] Create SCRATCHPAD.md working document
- [x] Set up Rust project structure
- [x] Implement arena allocator (basic)
- [x] Implement ART node types (Node4, Node16, Node48, Node256)
- [x] Implement ART insert/get/remove/prefix_scan
- [x] Create SimpleKV (BTreeMap-based) fallback
- [x] Fix ART Node48 restructuring bug
- [x] Box Node256 children array
- [x] Box Node48 child_index
- [x] Verify correctness with 1M URL dataset
- [x] **CompactArt**: Arena-backed key storage
- [x] **UltraCompactArt**: Arena-backed keys AND prefixes
- [x] **Add jemalloc for accurate memory measurement**
- [x] **ArenaArt**: Vec-based node storage (no Box overhead)

### FINAL RESULTS: Large-Scale Benchmark (9.5M URLs, 467 MB raw)

Using jemalloc's allocation tracking:

| Implementation | Total Memory | Overhead/Key | Notes |
|---------------|--------------|--------------|-------|
| **FrozenLayer (FST)** | **320 MB** | **-16 bytes (compression!)** | **WINNER for immutable** |
| BTreeMap | 1145 MB | 74.6 bytes | Best mutable structure |
| ArenaArt | 1449 MB | 108 bytes | Better than art-tree |
| art-tree crate | 1961 MB | 164 bytes | Existing Rust ART |

### Key Insights

1. **FST achieves 2.4x compression** - stores 467 MB of keys in 194 MB!
2. **BTreeMap is surprisingly efficient** - hard to beat for mutable data
3. **ART has inherent overhead** - 1.5 nodes per key minimum
4. **Allocation overhead matters** - ~48 bytes per Box in jemalloc

### Key Findings

1. **FST is the clear winner** for immutable/frozen data
   - 23 MB pure FST size (25.5 bytes/key)
   - 2x compression vs raw key data
   - 65% less memory than BTreeMap
   
2. **ART is not competitive** for moderate-size datasets with average-length keys
   - Too many nodes (1.46M for 967K keys)
   - Per-allocation overhead dominates (48 bytes/node in jemalloc)
   
3. **BTreeMap is highly optimized** and hard to beat with tree structures
   - High fanout (16-32 keys/node) → fewer nodes
   - Std library is extremely well optimized

### Recommendations

1. **For read-only/mostly data**: Use FST (FrozenLayer)
   - Extreme compression
   - Fast lookups and range queries
   - Must provide sorted input
   
2. **For mutable data with range queries**: Use BTreeMap or ART
   - BTreeMap is simpler and more memory efficient
   - ART has advantages for very long keys with high prefix sharing
   
3. **For hybrid workloads**: Use FST for frozen base + mutable delta
   - Periodic compaction merges delta into FST
   - Best of both worlds

### Optimizations Implemented

1. **FrozenLayer (FST)**: Extreme compression via Finite State Transducers
2. **ArenaArt**: Vec-based node storage (eliminates per-node allocation overhead)
3. **jemalloc integration**: Accurate memory measurement via tikv-jemalloc-ctl

### Attempted But Not Successful
- **art2 (OptimizedART)**: Stores only suffix in leaves instead of full key
  - Reduces internal memory to ~40 bytes/key
  - BUT has correctness bug with certain key patterns
  - Bug: after inserting key 231 "sighthound", ALL "s" prefixed keys return None
  - Works with isolated test cases but fails with full URL dataset
  - Documented for future investigation

- **SmallVec for prefixes**: Tried `SmallVec<[u8; 16]>` for inline prefix storage
  - Actually increased memory for URL dataset (most prefixes > 16 bytes)
  - Reverted

### Remaining Tasks  
- [ ] Child pointer compression (32-bit offsets in arena) - could reduce further
- [ ] FST integration for frozen layer - extreme compression for read-only data
- [ ] Performance optimization for Node48 slot reuse (currently O(n) search)

### Next Steps
1. Current 68 bytes/key is a 49% improvement over BTreeMap
2. Child pointer compression could reduce to ~50 bytes/key
3. FST layer would provide extreme compression for stable data

---

## Session Log

### Session 1: 2024-12-05 - Project Initialization & Implementation

**Goals**:
1. Create comprehensive documentation
2. Set up project structure
3. Begin ART implementation
4. Test with real data

**Progress**:
- Created DESIGN.md with full project specification
- Created SCRATCHPAD.md (this file)
- Implemented full ART with Node4/16/48/256
- Implemented arena allocator
- Implemented SimpleKV (BTreeMap-based) fallback
- Downloaded and tested with URL dataset (100K URLs)
- Verified correctness: 1000/1000 lookups pass

**Issues Found**:
1. ART has bug when handling >1000 keys - node restructuring corrupts tree
2. Specifically happens during Node4→Node16 or similar transitions
3. Keys sharing prefix become inaccessible after restructuring

**Decisions Made**:
- Use SimpleKV as interim backend for correctness
- Keep ART code for future debugging/optimization
- Focus on correctness first, then optimize

**Performance (SimpleKV with URL dataset)**:
- Insert: 19ms for 100K keys (~5M inserts/sec)
- Lookup: 1ms for 10K lookups (~10M ops/sec)
- Memory: ~113 bytes/key (similar to BTreeMap)

**Next Steps**:
1. Debug ART node restructuring bug
2. Implement arena-based key storage to reduce memory
3. Research FST for frozen layer compression

---

## Research Notes

### ART Implementation Details (from paper)

#### Key Insight: Adaptive Node Sizes
The genius of ART is using different node sizes based on actual fanout:
- Node4: 1-4 children → 4 slots
- Node16: 5-16 children → 16 slots (uses SIMD for search)
- Node48: 17-48 children → 256-entry index + 48 child pointers
- Node256: 49-256 children → direct indexing

#### Path Compression (Lazy Expansion)
Two strategies:
1. **Pessimistic**: Store partial keys at each node
2. **Optimistic**: Only store prefix length, check full key at leaf

For memory efficiency, optimistic is better but requires key storage.

#### Leaf Variants
Options for storing values:
1. Single value leaves
2. Multi-value leaves (for duplicate handling)
3. Combined (leaf can also be inner node)

For our case: Single value leaves, separate value arena.

### Memory Layout Experiments

#### Experiment 1: Pointer Size Impact
```
64-bit pointers: 8 bytes each
32-bit offsets: 4 bytes each (limited to 4GB arena)
```

With Node256 having 256 children:
- 64-bit: 2048 bytes
- 32-bit: 1024 bytes (50% savings!)

**Conclusion**: Use 32-bit offsets within arenas.

#### Experiment 2: Key Storage Strategies

Option A: All keys in arena
- Pro: Uniform handling
- Con: Pointer overhead for tiny keys

Option B: Inline small keys (≤16 bytes)
- Pro: Eliminates pointer for common case
- Con: Complexity, variable node sizes

Option C: Interned keys (deduplicated)
- Pro: Massive savings for repetitive data
- Con: Hash lookup overhead

**Initial Decision**: Option A for simplicity, consider B as optimization.

### FST Integration Ideas

The `fst` crate builds from sorted input. Options:

1. **Periodic rebuild**: Collect delta, sort, rebuild FST
   - Simple but expensive
   
2. **Streaming merge**: Merge-sort delta with existing FST
   - Efficient but need custom FST writer
   
3. **Multiple FSTs**: Keep old FST, build new for delta, merge later
   - Simple, some query overhead

**Initial Decision**: Option 3 for simplicity.

---

## Ideas & Brainstorms

### Memory Efficiency Ideas

1. **Compressed pointers**: Use relative offsets instead of absolute pointers
2. **Node pooling**: Pre-allocate node pages, use indices
3. **Lazy decompression**: Keep some data compressed until accessed
4. **Copy-on-write**: Share structure when possible

### Performance Ideas

1. **SIMD for Node16 search**: Use SSE/AVX to search 16 keys in parallel
2. **Prefetching**: Hint next nodes during traversal
3. **Branch prediction**: Optimize hot paths
4. **Cache-aligned nodes**: Align to cache line boundaries

### Alternative Approaches to Consider

1. **Learned indexes**: Train model on key distribution
   - Potentially huge savings for predictable distributions
   - Complex to implement, may not generalize
   
2. **Compressed B+Tree**: Like what databases use
   - Well-understood, good cache behavior
   - May be competitive with ART for memory

3. **HAT-Trie**: Hybrid hash + trie
   - Good cache locality
   - No natural ordering (problematic for range queries)

---

## Experiments Log

### Experiment: Baseline Memory Measurements

**Setup**: TODO
**Results**: TODO

### Experiment: ART vs BTreeMap Memory

**Setup**: TODO
**Results**: TODO

### Experiment: FST Compression Ratios

**Setup**: TODO
**Results**: TODO

---

## Primary Test Dataset: URL Crawl

**Source**: https://static.wilsonl.in/urls.txt
**Size**: ~16GB (~340 million URLs)

### Sample Analysis (first 100KB)
```
Lines: 2,078
Avg key length: 47.1 bytes
Min: 3 bytes, Max: 151 bytes
Format: Already sorted lexicographically
```

### Domain Distribution (sample)
```
432  0-www-elibrary-imf-org.library.svsu.edu
143  0-000000.tumblr.com
129  0-dear-rose-0.tumblr.com
 60  0-aredhel-0.tumblr.com
...
```

### Key Observations
1. **Extremely high prefix sharing** - hundreds of URLs per domain
2. **Hierarchical paths** - dates, post IDs, page numbers
3. **Pre-sorted** - perfect for FST construction
4. **Real-world distribution** - not synthetic

### Memory Targets for Full Dataset
| Approach | Bytes/Key | Total Memory |
|----------|-----------|--------------|
| BTreeMap naive | ~80 | ~27 GB |
| HashMap naive | ~70 | ~24 GB |
| Our target | ~10 | ~3.4 GB |
| Stretch goal | ~5 | ~1.7 GB |
| FST-level | ~3 | ~1 GB |

### Download Commands
```bash
# Full dataset (16GB - requires space and time)
curl -o urls_full.txt "https://static.wilsonl.in/urls.txt"

# Sample (first 1MB)
curl -r 0-1000000 "https://static.wilsonl.in/urls.txt" > urls_1mb.txt

# Sample (first 100MB) 
curl -r 0-100000000 "https://static.wilsonl.in/urls.txt" > urls_100mb.txt
```

---

## Issues & Solutions

### Issue 1: Node48 Slot Index Overflow
**Problem**: After ~1000 URL inserts, lookups would return None for previously inserted keys. The bug affected all keys sharing a common prefix.

**Analysis**: In Node48, children are stored in a Vec, and indices (0-255) are stored in `child_index[256]`. When remove_child is called followed by add_child for the same key byte:
1. `remove_child` sets `child_index[byte] = 255` and leaves a dummy in the Vec
2. `add_child` always pushes to the end of the Vec: `slot = children.len(); children.push(child)`
3. Over many remove/add cycles, `children.len()` grows beyond 255
4. `slot as u8` overflows, causing `child_index[byte] = 254` (or other wrong values)
5. `find_child` returns these corrupt indices, leading to wrong children being accessed

**Solution**: Modified `add_child` for Node48 to reuse existing dummy slots when `children.len() >= 48`, preventing unbounded Vec growth.

**Status**: Fixed. All 966,956 URL lookups now succeed (100% correctness). 

---

## Code Snippets & Prototypes

### Arena Allocator Sketch
```rust
pub struct Arena {
    chunks: Vec<Box<[u8]>>,
    current: *mut u8,
    remaining: usize,
}

impl Arena {
    pub fn alloc(&mut self, size: usize) -> *mut u8 {
        if size > self.remaining {
            self.grow(size);
        }
        let ptr = self.current;
        self.current = unsafe { self.current.add(size) };
        self.remaining -= size;
        ptr
    }
}
```

### Node4 Sketch
```rust
#[repr(C)]
pub struct Node4 {
    pub header: NodeHeader,      // 8 bytes
    pub keys: [u8; 4],           // 4 bytes
    pub children: [u32; 4],      // 16 bytes (arena offsets)
}
// Total: 28 bytes, padded to 32
```

---

## Benchmark Results

### 2024-12-05 - URL Dataset (100K URLs)

**Dataset**: Real URLs from web crawl, ~5MB, 100K keys
**Average key length**: 49 bytes
**Test environment**: Release build

| Metric | MemKV (SimpleKV) | BTreeMap | Target |
|--------|------------------|----------|--------|
| Memory (bytes/key) | ~113 | ~133 | <10 |
| Insert (ops/sec) | ~5M | ~5M | >100K |
| Lookup (ops/sec) | ~10M | ~10M | >100K |
| Memory overhead | 2.3x raw | 2.7x raw | <1.5x |

### 2024-12-05 - Synthetic Sequential Keys (100K)

**Dataset**: format!("user:{:08}", i), ~17 bytes avg

| Metric | MemKV (SimpleKV) | BTreeMap |
|--------|------------------|----------|
| Memory (bytes/key) | ~85 | ~107 |
| Insert (ops/sec) | ~5M | ~5M |
| Lookup (ops/sec) | ~10M | ~10M |

---

## References Consulted

- [ ] ART paper (full read)
- [ ] HOT paper
- [ ] FST crate source code
- [ ] RocksDB memtable implementation
- [ ] DuckDB ART implementation

---

## Questions & Open Items

1. **Q**: Should we support delete or just tombstone?
   **A**: TBD - tombstone simpler, delete more memory efficient long-term

2. **Q**: How to handle extremely long keys (>1KB)?
   **A**: TBD - likely separate storage with hash for lookup

3. **Q**: Persistence format?
   **A**: Deferred - focus on in-memory first

4. **Q**: What's the right delta layer size before compaction?
   **A**: TBD - experiment needed, likely 1-10% of total

---

## Decision Log

| Date | Decision | Rationale | Status |
|------|----------|-----------|--------|
| 2024-12-05 | Use hybrid ART+FST | Best of both worlds | Active |
| 2024-12-05 | 32-bit offsets | 50% pointer savings | Planned |
| 2024-12-05 | RwLock concurrency | Simple, sufficient | Planned |

---

## Performance Profiling Notes

### Hotspots Identified
(To be filled during profiling)

### Optimization Opportunities
(To be filled during profiling)

---

## Weekly Goals

### Week 1 (DONE)
- [x] Project setup
- [x] Arena allocator (basic)
- [x] Basic ART nodes (Node4, Node16, Node48, Node256)
- [x] Basic insert/get
- [x] SimpleKV fallback implementation
- [x] Test with real URL data

### Week 2 (Next)
- [ ] Debug ART node restructuring bug
- [ ] Add proper memory tracking
- [ ] Arena-based key storage
- [ ] Compare with radix_trie crate

### Week 3
- [ ] Memory optimizations
- [ ] Pointer compression (32-bit offsets)
- [ ] Comparison benchmarks

### Week 4
- [ ] FST integration
- [ ] Hybrid queries
- [ ] Compaction

### Week 5
- [ ] Concurrency improvements
- [ ] Polish
- [ ] Final benchmarks

## ART Bug Investigation

### Symptoms
- After ~1000 insertions, keys become inaccessible
- Specifically affects keys sharing prefix when node restructuring happens
- Example: Inserting key 1062 corrupts keys 974-1061 (all share "0-r" prefix)

### Hypothesis
- Bug in node growth (Node4→Node16 or Node16→Node48)
- Or bug in remove_child + add_child pair during insertion
- Tree corruption when node transitions

### Debug Strategy
1. Add detailed logging to node growth functions
2. Trace exact state before/after corruption
3. Simplify test case to minimal reproduction
4. Check similar implementations (art-rs, DuckDB) for comparison

---

## Miscellaneous Notes

### Useful Commands
```bash
# Run benchmarks
cargo bench

# Check memory usage
cargo run --release --example memory_test

# Profile with perf
perf record -g cargo run --release --example profile_test
perf report

# Check for memory leaks
valgrind --leak-check=full ./target/release/examples/memory_test
```

### Environment Setup
```bash
# Install Rust nightly (for some optimizations)
rustup install nightly

# Useful tools
cargo install cargo-criterion
cargo install cargo-flamegraph
```

