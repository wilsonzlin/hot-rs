# MemKV Development Scratchpad

> This is a living document for tracking progress, experiments, ideas, and issues.
> Updated continuously throughout development.

---

## Current Status

**Phase**: Initial Planning & Research
**Date Started**: 2024-12-05
**Last Updated**: 2024-12-05

### Active Tasks
- [x] Create DESIGN.md reference document
- [x] Create SCRATCHPAD.md working document
- [ ] Set up Rust project structure
- [ ] Implement arena allocator
- [ ] Implement basic ART nodes

### Blockers
None currently.

---

## Session Log

### Session 1: 2024-12-05 - Project Initialization

**Goals**:
1. Create comprehensive documentation
2. Set up project structure
3. Begin ART implementation

**Progress**:
- Created DESIGN.md with full project specification
- Created SCRATCHPAD.md (this file)

**Notes**:
- Decided on hybrid ART + FST architecture
- Primary focus on memory efficiency

**Next Steps**:
- Initialize Cargo project
- Set up benchmarking infrastructure
- Begin arena allocator

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

## Issues & Solutions

### Issue 1: [Template]
**Problem**: 
**Analysis**:
**Solution**:
**Status**: 

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

### [Date] - [Description]

| Metric | Our Implementation | BTreeMap | HashMap |
|--------|-------------------|----------|---------|
| Memory (bytes/key) | | | |
| Insert (ops/sec) | | | |
| Lookup (ops/sec) | | | |
| Range (ops/sec) | | | |

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

### Week 1 (Current)
- [ ] Project setup
- [ ] Arena allocator
- [ ] Basic ART nodes
- [ ] Basic insert/get

### Week 2
- [ ] Full ART operations
- [ ] Iteration
- [ ] Initial benchmarks

### Week 3
- [ ] Memory optimizations
- [ ] Pointer compression
- [ ] Comparison benchmarks

### Week 4
- [ ] FST integration
- [ ] Hybrid queries
- [ ] Compaction

### Week 5
- [ ] Concurrency
- [ ] Polish
- [ ] Final benchmarks

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

