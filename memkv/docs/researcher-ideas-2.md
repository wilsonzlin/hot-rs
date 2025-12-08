# Memory-efficient key-value indexes for billion-key Rust stores

**HOT (Height Optimized Trie) emerges as the optimal choice for your requirements**, achieving **11-14 bytes per key** regardless of key distribution—roughly half the memory of ART and competitive B+ trees. For prefix-heavy string keys like file paths and URLs at billion-key scale, HOT would consume approximately **14GB of index overhead** versus 25-30GB for ART. However, HOT requires C++ FFI integration since no Rust implementation exists. A practical alternative is an **optimized ART implementation** combined with aggressive memory techniques (pointer compression, arena allocation, front coding) which can achieve **15-20 bytes per key** with available Rust crates.

## Memory overhead comparison across structures

The critical decision hinges on achievable bytes-per-key at billion scale with range query support:

| Structure | Bytes/Key (String Keys) | Range Queries | Updates | Rust Ready |
|-----------|------------------------|---------------|---------|------------|
| **HOT** | **11-14** | ✅ Excellent | ✅ Yes | ❌ FFI required |
| **ART** | 15-30 typical, 52 max | ✅ Good | ✅ Yes | ✅ Multiple crates |
| **FST** | ~1.25 (bits/node) | ✅ Excellent | ❌ Static | ✅ fst-rs |
| **Judy arrays** | 5-7 (sequential) | ✅ Yes | ✅ Yes | ⚠️ Partial |
| **B+ tree + prefix** | 20-40 | ✅ Excellent | ✅ Yes | ✅ sled |
| **patricia_tree** | ~32 (measured) | ⚠️ Prefix only | ✅ Yes | ✅ Production |
| **Masstree** | 25-35 | ✅ Good | ✅ Yes | ❌ None mature |

Your requirement of **~100K ops/sec** is easily achievable by any of these structures—modern implementations hit millions of ops/sec. Memory efficiency genuinely differentiates them.

## HOT delivers the best memory efficiency for string keys

The 2018 SIGMOD paper demonstrated HOT's superiority across diverse workloads. Its key innovation is **dynamically varying the span per node** rather than fixed 8-bit spans like ART. By combining multiple Patricia trie levels into compound nodes with maximum fanout of 32, HOT maintains consistently high fanout regardless of data distribution.

For **URLs averaging 55 bytes**, HOT achieves **14.4 bytes per key** versus ART's 20-30 bytes—a **50% reduction**. The memory consistency is particularly valuable: HOT's range of 11.4-14.4 bytes/key across all tested workloads contrasts sharply with ART's 8-52 byte range depending on key distribution.

HOT uses **SIMD (AVX2) and BMI2 (PEXT/PDEP)** instructions for parallel key comparison within compound nodes. The concurrent variant implements ROWEX (Read-Optimized Write EXclusion) achieving near-linear scalability—9.96x speedup with 10 threads—with wait-free readers.

**Implementation path**: The reference C++ implementation at `github.com/speedskater/hot` (ISC license) requires wrapping via FFI. Creating a pure Rust implementation would be a significant undertaking given the 9 physical node layouts and SIMD optimizations, but would be valuable for the ecosystem.

## ART provides the best practical Rust option today

For a native Rust solution, the **Adaptive Radix Tree** offers the best balance of memory efficiency, range query support, and implementation maturity. ART's four adaptive node types (Node4/16/48/256) dynamically resize based on occupancy, with path compression collapsing single-child chains.

**Memory characteristics** for your workload:
- File paths, URLs, hierarchical keys: **20-30 bytes/key** typical
- Worst-case guarantee: **52 bytes/key** for arbitrarily long keys
- Best case (dense sequential): **8.1 bytes/key**

The **`rart` crate** provides the most complete implementation with SIMD optimizations and copy-on-write snapshots for concurrent workloads. The **`congee` crate** implements ART-OLC (Optimistic Lock Coupling) achieving **150M ops/sec on 32 cores** but is limited to 8-byte fixed keys. For variable-length string keys, `rart` or `art-tree` with custom modifications would be necessary.

Range queries work naturally since ART maintains lexicographic order by design. Iterators support range scans, prefix lookups, and min/max operations. The structure requires binary-comparable key transformation (byte-order preservation) which your key patterns naturally satisfy.

## Succinct structures offer extreme compression with trade-offs

**FST (Fast Succinct Tries)** achieves approximately **10 bits per trie node**—close to the information-theoretic minimum of 9.44 bits. The `fst` Rust crate (formerly `fst-rs`) provides a production-quality implementation. RocksDB uses FST internally for prefix bloom filters.

However, FST is **static-only**—construction requires a sorted key-value list and produces an immutable structure. For your in-memory KV store loading data at startup, this could work: build the FST from your persisted WAL/log during initialization. Updates would require a **hybrid approach**: maintain a small dynamic write buffer (skiplist or ART) alongside the static FST, periodically merging.

**SuRF (Succinct Range Filters)** extends FST for approximate range queries at **10-14 bits per key**. It stores truncated keys (minimum distinguishing prefix + suffix bits) rather than full keys, producing false positives. This suits filter use cases but not primary storage.

The fundamental trade-off: **succinctness requires precomputation**. You cannot simultaneously have O(1) query time, near-optimal space, and efficient updates.

## Hybrid architecture maximizes memory efficiency

Given your constraints (billions of keys, memory-first priority, range queries, read-heavy, startup loading), a **hybrid architecture** offers the best practical solution:

```
┌─────────────────────────────────────────────────────────────┐
│                    Write Buffer (Hot)                        │
│  Small ART or Skiplist • Latest writes • ~1-10MB            │
└────────────────────────┬────────────────────────────────────┘
                         │ Periodic merge
┌────────────────────────▼────────────────────────────────────┐
│                    Main Index (Cold)                         │
│  FST or HOT • Bulk-loaded at startup • Billions of keys     │
│  ~10-14 bytes/key • Immutable between merges                │
└─────────────────────────────────────────────────────────────┘
```

**Reads** check the write buffer first, then the main index. **Writes** go to the buffer. Periodically, merge buffer into main index (rebuild FST or update HOT). This mirrors LSM-tree architecture but optimized for memory rather than disk.

For your read-optimized workload, the main index handles 99%+ of operations. The merge frequency depends on write rate—with single-writer mutex semantics, even simple periodic rebuilds work well.

## Memory optimization techniques provide additional 30-50% savings

Apply these techniques regardless of primary structure choice:

**Pointer compression** reduces 64-bit pointers to 32-bit offsets when data fits within 4GB. V8's implementation achieves **40% memory reduction** for pointer-heavy structures. For a 14GB index, store 32-bit offsets from a base address rather than full pointers. With 8-byte alignment, you gain 3 extra bits—addressing 32GB with 32-bit values.

```rust
// Compressed pointer: offset from base, assuming 8-byte alignment
fn compress(ptr: *const u8, base: *const u8) -> u32 {
    ((ptr as usize - base as usize) >> 3) as u32
}
fn decompress(offset: u32, base: *const u8) -> *const u8 {
    unsafe { base.add((offset as usize) << 3) }
}
```

**Arena allocation** eliminates per-allocation overhead (typically 8-16 bytes per malloc). Use `bumpalo` for phase-oriented allocation where you allocate during index construction and deallocate everything together. For persistent structures, `typed-arena` provides single-type arenas with destructor support.

**Front coding (prefix compression)** for sorted keys stores common prefix once, then unique suffixes. HBase benchmarks show **25-80% key compression** depending on prefix sharing. Your hierarchical key patterns (paths, URLs) are ideal candidates:

```
document/1/author/1  →  [prefix: "document/1/author/"] + "1"
document/1/author/2  →  [shared prefix ref] + "2"
document/1/author/3  →  [shared prefix ref] + "3"
```

**Varint encoding** for lengths and small integers saves 60%+ space. Use LEB128 (Protocol Buffers style) or the faster `vu128` encoding which uses a length-prefix byte instead of per-byte continuation bits.

## Rust ecosystem assessment and gaps

**Production-ready options for your use case:**

| Crate | Memory Efficiency | Range Queries | Notes |
|-------|-------------------|---------------|-------|
| `patricia_tree` | Excellent (56-80% vs HashMap) | Prefix-based | Actively maintained, 885 dependents |
| `fst` | Exceptional | Full support | Static only, BurntSushi maintained |
| `im` (OrdMap) | Good (structural sharing) | Full support | Mature, 15.1.0, widely used |
| `rart` | Good (ART design) | Full support | SIMD optimized, versioned |

**Gaps requiring custom implementation:**
1. **HOT in Rust**: Highest-value target—would provide best memory efficiency
2. **Variable-length keys with both range queries AND memory efficiency**: Most crates optimize for one
3. **Inline value storage**: Most tries store values separately; truly inline storage requires custom work
4. **SIMD-optimized pure Rust ART**: Existing implementations don't fully exploit SIMD

The **`patricia_tree` crate** deserves special attention. Benchmarks show it achieves **424MB for 13.4M Wikipedia titles versus 978MB for HashSet**—a 56% reduction. For Google 5-gram data, it achieves 80% reduction. The trade-off is ~6-7x slower insertions, but your read-optimized workload makes this acceptable.

## Concrete recommendations ranked by memory efficiency

**Tier 1 (Best Memory): Custom Implementation or FFI**
1. **HOT via C++ FFI**: 11-14 bytes/key. Wrap `speedskater/hot` using `bindgen`. Highest effort but best results.
2. **FST + dynamic buffer hybrid**: ~10 bits/node for static portion. Use `fst` crate with small ART buffer for writes.

**Tier 2 (Good Memory): Native Rust with Optimization**
3. **Optimized ART (`rart` or custom)**: 15-25 bytes/key with pointer compression and arena allocation. Apply all memory optimization techniques.
4. **`patricia_tree` with custom allocator**: ~32 bytes/key baseline, reducible with arena allocation and front coding applied at application layer.

**Tier 3 (Baseline): Production-Ready Today**
5. **`im::OrdMap`**: B-tree with structural sharing. Higher memory but excellent API, range queries, persistent snapshots.
6. **B+ tree with prefix compression (`sled` internals)**: Proven at scale, well-understood characteristics.

## Implementation roadmap for maximum efficiency

**Phase 1: Validate with existing crates (1-2 weeks)**
- Benchmark `patricia_tree` and `rart` with representative data
- Measure actual bytes/key with your key distribution
- Confirm range query performance meets requirements

**Phase 2: Apply memory optimization techniques (2-4 weeks)**
- Implement pointer compression layer
- Integrate arena allocation (`bumpalo`)
- Add front coding for sorted key storage

**Phase 3: Custom implementation or FFI (4-8 weeks)**
- Option A: Wrap HOT C++ implementation with Rust FFI
- Option B: Implement optimized ART from scratch with all techniques
- Option C: Build FST+buffer hybrid architecture

**Phase 4: Production hardening (ongoing)**
- Implement efficient bulk loading for startup
- Add snapshotting support (copy-on-write or serialization)
- Benchmark at billion-key scale

## Cutting-edge research worth monitoring

**LITS (2024)**: Hash-enhanced Prefix Table + sub-tries specifically for string keys. Early results show potential to outperform ART/HOT for certain distributions, though traditional indexes still win overall.

**Memento Filter (SIGMOD 2025)**: First dynamic range filter supporting inserts/deletes. Built on quotient filters, integrated into WiredTiger with 2× throughput improvement. Could enable new hybrid architectures.

**Compressed ART variants**: Research continues on reducing ART's worst-case bounds through better node layouts and compression schemes.

**PGM-index for sorted integers**: If you can map string keys to order-preserving integers, PGM achieves 83-1140× less space than B-trees. However, the mapping overhead typically negates benefits for string keys.

## Final verdict

For billion-key prefix-heavy string workloads prioritizing memory efficiency: **invest in HOT FFI integration or build a hybrid FST + ART architecture**. The 50% memory savings over standard ART translates to ~10-15GB at your scale—significant infrastructure cost reduction.

If development time is constrained, start with **`patricia_tree`** (production-ready, 56-80% compression) while building toward HOT. Apply pointer compression, arena allocation, and front coding regardless of primary structure choice—these techniques compound to provide an additional 30-50% reduction.

Your performance target of 100K ops/sec is trivially achievable; the real differentiation is memory efficiency, where HOT's consistent 11-14 bytes/key makes it the clear winner for your workload characteristics.
