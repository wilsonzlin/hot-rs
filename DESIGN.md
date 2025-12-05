# Memory-Efficient Key-Value Store for Billions of Keys

## Project Overview

**Goal**: Create a Rust library for storing and querying string keys (UTF-8 or arbitrary bytes) with arbitrary values, optimized for extreme memory efficiency at billion-key scale.

**Primary Constraint**: Memory efficiency (bytes per key-value pair)
**Secondary Constraint**: Reasonable performance (~100K ops/sec)
**Operations Required**: Point lookups, range queries, insertions

---

## Table of Contents

1. [Problem Analysis](#problem-analysis)
2. [Requirements Deep Dive](#requirements-deep-dive)
3. [Data Structure Survey](#data-structure-survey)
4. [Algorithm Research](#algorithm-research)
5. [Existing Implementations Analysis](#existing-implementations-analysis)
6. [Design Decisions](#design-decisions)
7. [Architecture](#architecture)
8. [Implementation Plan](#implementation-plan)
9. [Benchmarking Strategy](#benchmarking-strategy)
10. [References & Papers](#references--papers)

---

## 1. Problem Analysis

### Why Traditional Approaches Fail at Scale

#### BTreeMap Memory Overhead

A `BTreeMap<String, V>` in Rust has significant overhead:
- **String allocation**: 24 bytes (ptr + len + capacity) + actual string data on heap
- **BTreeMap node overhead**: Internal nodes with multiple keys, child pointers
- **Fragmentation**: Each String is a separate heap allocation
- **No prefix sharing**: "user:123" and "user:124" store full strings separately

For a billion keys averaging 32 bytes each:
- Naive: ~56 bytes/key minimum (24 + 32) = 56GB just for keys
- With node overhead and fragmentation: potentially 80-100+ GB

#### HashMap Memory Overhead
- Similar String overhead
- Hash table load factor (typically 50-87.5% efficient)
- Each bucket has 8+ bytes of metadata
- No range query support

### What "Extreme Memory Efficiency" Means

**Target**: Get as close to the **information-theoretic minimum** as possible.

For N keys of average length L bytes:
- Raw data: N × L bytes
- But we can do better with:
  - **Prefix sharing**: Common prefixes stored once
  - **Succinct encoding**: Near-optimal bit representations
  - **Compression**: Entropy coding, dictionary compression

**Benchmark comparison points**:
| Approach | Bytes/Key (32-byte avg key, no value) |
|----------|---------------------------------------|
| BTreeMap<String, ()> | 60-80 bytes |
| HashMap<String, ()> | 50-70 bytes |
| Trie (naive) | 40-60 bytes |
| Radix/Patricia Trie | 20-40 bytes |
| Adaptive Radix Tree | 10-20 bytes |
| FST (immutable) | 2-5 bytes |
| Succinct Trie | 1-3 bytes |

---

## 2. Requirements Deep Dive

### Functional Requirements

1. **Key Types**
   - UTF-8 strings (most common)
   - Arbitrary byte sequences (for binary keys)
   - Variable length keys (1 byte to ~64KB realistically)

2. **Value Types**
   - Generic `V` type parameter
   - For DB use: likely u64 offsets, small structs, or serialized data
   - Value size affects memory strategy significantly

3. **Operations**
   - `insert(key, value)` → Option<V> (old value if existed)
   - `get(key)` → Option<&V>
   - `contains(key)` → bool
   - `remove(key)` → Option<V>
   - `range(start..end)` → Iterator<(Key, V)>
   - `prefix_scan(prefix)` → Iterator<(Key, V)>
   - `len()` → usize
   - `is_empty()` → bool

4. **Ordering**
   - Lexicographic byte ordering for range queries
   - Must be deterministic and consistent

### Non-Functional Requirements

1. **Memory Efficiency** (PRIMARY)
   - Target: <10 bytes overhead per key for typical workloads
   - Ideally: approach FST-level efficiency (~2-5 bytes/key) while remaining mutable

2. **Performance**
   - Target: 100K+ ops/sec for point queries
   - Insertion can be slower (10-100K/sec acceptable)
   - Range queries: proportional to result set size

3. **Concurrency**
   - Minimum: Single-writer, multiple-reader safe
   - Nice-to-have: Concurrent insertions
   - Acceptable: Mutex-guarded writes if memory efficiency preserved

4. **Ergonomics**
   - Clean, idiomatic Rust API
   - No unsafe in public API (internal unsafe acceptable if justified)
   - Good error handling

5. **Persistence** (Future consideration)
   - Memory-mappable representation ideal
   - Serialization support

### Key Workload Assumptions

Based on database key patterns:
1. **High prefix sharing**: Keys often have common prefixes (table names, timestamps, user IDs)
2. **Clustered insertions**: Keys often inserted in near-sorted order
3. **Read-heavy**: Queries typically outnumber writes
4. **Skewed access**: Some keys accessed far more than others (can inform caching)

---

## 3. Data Structure Survey

### 3.1 Trie Family

#### Classic Trie
- **Structure**: Each node has up to 256 children (for byte keys)
- **Memory**: Terrible - 256 pointers per node = 2KB on 64-bit
- **Lookup**: O(key_length)
- **Not viable** for our use case

#### Patricia Trie (Radix Tree)
- **Key insight**: Compress chains of single-child nodes
- **Structure**: Edges labeled with substrings, not single characters
- **Memory**: Much better - O(n) nodes for n keys
- **Lookup**: O(key_length)
- **Range queries**: Natural traversal
- **Mutable**: Yes, though rebalancing needed
- **Viable candidate**

#### Adaptive Radix Tree (ART)
- **Paper**: "The Adaptive Radix Tree: ARTful Indexing for Main-Memory Databases" (2013)
- **Key insight**: Adapt node size to fanout (4, 16, 48, 256 children variants)
- **Memory**: Excellent - adapts to actual usage
- **Lookup**: O(key_length), but faster constants than Patricia
- **Cache efficiency**: Designed for modern CPUs
- **Used by**: DuckDB, many modern in-memory databases
- **Strong candidate**

#### HAT-Trie
- **Paper**: "HAT-trie: A Cache-conscious Trie-based Data Structure" (2007)
- **Key insight**: Hybrid of trie + hash table at leaves
- **Memory**: Very good for short keys
- **Cache efficiency**: Optimized for L1/L2 cache
- **Consideration**: More complex to implement

#### Burst Trie
- **Key insight**: Containers at leaves that "burst" into trie nodes when too large
- **Memory**: Adaptive based on key distribution
- **Good for**: Skewed key distributions

### 3.2 Succinct Data Structures

#### What is "Succinct"?
A data structure is succinct if it uses space close to the information-theoretic minimum while supporting efficient operations.

For n keys: minimum space = n × average_key_length bits
Succinct target: minimum + o(minimum) extra bits

#### LOUDS (Level-Order Unary Degree Sequence)
- **Encoding**: Trie structure in ~2n bits for n nodes
- **Operations**: O(1) with rank/select support
- **Memory**: Exceptional
- **Drawback**: Static (rebuilding required for modifications)

#### Succinct Trie Implementations
- **FST (Finite State Transducer)**: Used by tantivy, Lucene
- **Marisa-trie**: Compressed trie with LOUDS
- **XCDAT**: Compressed double-array trie

### 3.3 Finite State Transducers (FST)

#### What is an FST?
- Generalization of finite state automata that produces output
- Maps keys → values with shared structure
- **Immutable** but extremely compact

#### Properties
- **Space**: Often 10-50% of raw key data
- **Lookup**: O(key_length), very fast in practice
- **Construction**: Requires sorted input, O(n) time
- **Range queries**: Excellent via automaton traversal
- **Drawback**: Immutable - must rebuild for modifications

#### The `fst` Crate (Rust)
- Mature, well-optimized implementation
- Used in production (tantivy search engine)
- Values limited to u64

### 3.4 Other Approaches

#### B+ Trees with Prefix Compression
- **Database standard**: PostgreSQL, MySQL, RocksDB
- **Prefix compression**: Share common prefixes within nodes
- **Suffix truncation**: Store minimal distinguishing prefix
- **Cache-efficient**: Designed for disk but works well in memory

#### Judy Arrays
- **Highly optimized**: 256-way digital tree with compression
- **Memory**: Very good
- **Implementation**: Extremely complex (original is 20K lines of C)
- **Rust crate**: `judy` exists but limited

#### HAMT (Hash Array Mapped Trie)
- **Used by**: Clojure, Scala immutable collections
- **Hybrid**: Hash + trie structure
- **Good for**: Persistent/immutable data structures
- **Drawback**: No natural ordering for range queries

---

## 4. Algorithm Research

### 4.1 Compression Techniques

#### Front Coding (Prefix Compression)
Store keys as (shared_prefix_length, suffix):
```
"user:alice" → (0, "user:alice")
"user:bob"   → (5, "bob")
"user:carol" → (5, "carol")
```
**Savings**: Proportional to prefix sharing

#### Delta Encoding
For sorted keys, store differences:
- Works well with front coding
- Enables block compression

#### Dictionary Compression
For repeated substrings:
- Build dictionary of common patterns
- Replace with short codes
- LZ4, Snappy, ZSTD at block level

#### Variable-Length Integer Encoding
For values (offsets, lengths):
- VarInt: 1-10 bytes for u64
- Group Varint: Batch 4 integers efficiently
- PForDelta: For sorted integer sequences

### 4.2 Hybrid Approaches

#### Frozen + Delta Architecture
Like LSM-trees but for indexes:
1. **Frozen layer**: Immutable, succinct (FST-like)
2. **Delta layer**: Mutable, less efficient (ART/BTree)
3. **Periodic compaction**: Merge delta into new frozen layer

**Trade-offs**:
- Reads: Check delta first, then frozen
- Writes: Fast (to delta layer)
- Memory: Amortized good (most data in frozen)
- Compaction: Background CPU cost

#### Tiered Approach
Multiple tiers with different characteristics:
1. **Hot tier**: Small, mutable, fast
2. **Warm tier**: Medium, less mutable
3. **Cold tier**: Large, immutable, compact

### 4.3 Memory Layout Optimization

#### Arena Allocation
- Allocate keys in contiguous memory regions
- Reduces fragmentation
- Improves cache locality
- Enables efficient serialization

#### String Interning
- Deduplicate common substrings
- Good for highly repetitive data
- Adds lookup overhead

#### Pointer Compression
- Use 32-bit offsets instead of 64-bit pointers (within arenas)
- Saves 50% on pointer-heavy structures
- Limits addressable space to 4GB per arena

---

## 5. Existing Implementations Analysis

### 5.1 Rust Crates

#### `fst` (Finite State Transducer)
- **Maturity**: Production-ready
- **Memory**: Exceptional
- **API**: Builder pattern, immutable once built
- **Values**: u64 only
- **Verdict**: Excellent for static data, baseline for comparison

#### `radix_trie`
- **Memory**: Moderate
- **API**: Standard mutable interface
- **Verdict**: Simple but not optimized for extreme efficiency

#### `patricia_tree`
- **Focus**: IP routing tables
- **Verdict**: Specialized, not general-purpose

#### `art` / `art-rs`
- **Based on**: ART paper
- **Maturity**: Less mature
- **Verdict**: Worth evaluating

#### `qp-trie`
- **Based on**: QP-Trie (nibble-based)
- **Verdict**: Interesting alternative

### 5.2 Other Languages/Systems

#### DuckDB's ART Implementation
- Modern, well-optimized
- Includes leaf compression techniques
- C++ but readable

#### LMDB's B+Tree
- Memory-mapped
- Copy-on-write
- Excellent for persistence

#### RocksDB's MemTable
- Skiplist default
- Hash variants available
- Good reference for concurrent access

#### SQLite's B-Tree
- Extremely well-tested
- Sophisticated prefix compression

### 5.3 Academic Implementations

#### HOPE (High-speed Order-Preserving Encoder)
- Paper: "HOPE: A New Entropy Encoder for Efficient Key-Value Storage" (2020)
- Compresses keys while preserving order
- Enables compressed comparisons

#### SuRF (Succinct Range Filter)
- Paper: "SuRF: Practical Range Query Filtering" (2018)
- Succinct + range queries
- Used as filter, not primary index

---

## 6. Design Decisions

### Decision 1: Core Data Structure

**Chosen Approach**: Hybrid Adaptive Radix Tree + Frozen FST

**Rationale**:
1. ART provides excellent mutable performance with good memory efficiency
2. FST provides exceptional memory efficiency for stable data
3. Hybrid allows amortized excellent memory with fast mutations

**Architecture**:
```
┌─────────────────────────────────────────────┐
│              MemoryKV API                    │
├─────────────────────────────────────────────┤
│  ┌─────────────┐    ┌─────────────────────┐ │
│  │ Delta Layer │    │    Frozen Layer     │ │
│  │    (ART)    │    │       (FST)         │ │
│  │  - Mutable  │    │    - Immutable      │ │
│  │  - Fast ins │    │    - Compact        │ │
│  └─────────────┘    └─────────────────────┘ │
├─────────────────────────────────────────────┤
│           Background Compactor               │
└─────────────────────────────────────────────┘
```

### Decision 2: Value Storage Strategy

**Options Considered**:
1. Store values inline in trie nodes
2. Store value offsets, actual values in arena
3. Store only in leaves (no internal node values)

**Chosen**: Option 2 - Offset + Arena
- Keys and values in separate arenas
- Nodes store 4-byte offsets (pointer compression)
- Enables large values without bloating nodes

### Decision 3: Concurrency Model

**Chosen**: Single-writer, multi-reader with RwLock

**Rationale**:
- Simpler implementation
- Sufficient for most database workloads
- Memory efficiency preserved

**Future**: Lock-free reads with epoch-based reclamation

### Decision 4: Key Representation

**Chosen**: Byte slices with optimized storage

```rust
// Keys stored in arenas
// Small keys: inline in nodes (<= 16 bytes)
// Large keys: offset into key arena
```

---

## 7. Architecture

### Module Structure

```
src/
├── lib.rs              # Public API
├── art/                # Adaptive Radix Tree implementation
│   ├── mod.rs
│   ├── node.rs         # Node types (Node4, Node16, Node48, Node256)
│   ├── tree.rs         # Tree operations
│   └── iter.rs         # Iterators
├── fst_layer/          # FST frozen layer
│   ├── mod.rs
│   ├── builder.rs      # FST construction
│   └── reader.rs       # FST queries
├── hybrid/             # Combined structure
│   ├── mod.rs
│   ├── compaction.rs   # Background compaction
│   └── merge.rs        # Delta + Frozen merging
├── arena/              # Memory arenas
│   ├── mod.rs
│   ├── key_arena.rs
│   └── value_arena.rs
├── encoding/           # Key/value encoding
│   ├── mod.rs
│   ├── varint.rs
│   └── prefix.rs
└── bench/              # Internal benchmarks
```

### Public API

```rust
pub struct MemKV<V> { ... }

impl<V: Clone> MemKV<V> {
    /// Create new empty store
    pub fn new() -> Self;
    
    /// Create with configuration
    pub fn with_config(config: Config) -> Self;
    
    /// Insert a key-value pair
    pub fn insert(&self, key: impl AsRef<[u8]>, value: V) -> Option<V>;
    
    /// Get value for key
    pub fn get(&self, key: impl AsRef<[u8]>) -> Option<&V>;
    
    /// Check if key exists
    pub fn contains(&self, key: impl AsRef<[u8]>) -> bool;
    
    /// Remove a key
    pub fn remove(&self, key: impl AsRef<[u8]>) -> Option<V>;
    
    /// Iterate over range
    pub fn range<R: RangeBounds<[u8]>>(&self, range: R) -> Range<'_, V>;
    
    /// Iterate with prefix
    pub fn prefix(&self, prefix: impl AsRef<[u8]>) -> Prefix<'_, V>;
    
    /// Number of keys
    pub fn len(&self) -> usize;
    
    /// Memory usage statistics
    pub fn memory_usage(&self) -> MemoryStats;
    
    /// Force compaction
    pub fn compact(&self);
}
```

---

## 8. Implementation Plan

### Phase 1: Foundation (Week 1)
- [ ] Project setup (Cargo.toml, CI, benchmarks infrastructure)
- [ ] Arena allocator implementation
- [ ] Basic ART node types (Node4, Node16, Node48, Node256)
- [ ] Basic ART operations (insert, get)
- [ ] Initial benchmarks

### Phase 2: Full ART (Week 2)
- [ ] ART remove operation
- [ ] ART iteration (in-order)
- [ ] ART range queries
- [ ] Path compression (lazy expansion)
- [ ] Leaf page compression

### Phase 3: Memory Optimization (Week 3)
- [ ] Pointer compression (32-bit offsets)
- [ ] Key inlining for small keys
- [ ] Memory profiling and optimization
- [ ] Comparison benchmarks vs BTreeMap

### Phase 4: FST Integration (Week 4)
- [ ] FST builder from sorted iterator
- [ ] FST reader integration
- [ ] Hybrid query logic (delta + frozen)
- [ ] Basic compaction

### Phase 5: Polish & Performance (Week 5)
- [ ] Concurrency (RwLock integration)
- [ ] Advanced compaction strategies
- [ ] Edge case handling
- [ ] Documentation
- [ ] Comprehensive benchmarks

---

## 9. Benchmarking Strategy

### Metrics to Track

1. **Memory Efficiency**
   - Bytes per key-value pair
   - Overhead ratio vs raw data size
   - Memory fragmentation

2. **Throughput**
   - Point lookups per second
   - Range query throughput
   - Insert throughput
   - Mixed workload

3. **Latency**
   - P50, P99, P99.9 for operations
   - Tail latency during compaction

### Benchmark Datasets

1. **Synthetic - Random**
   - Random 16-byte keys
   - Random 32-byte keys
   - Random length (8-64 bytes)

2. **Synthetic - Structured**
   - UUID-like keys
   - Timestamp-prefixed keys
   - Hierarchical paths (a/b/c/d)

3. **Real-world: URL Dataset** ⭐ PRIMARY
   - **Source**: https://static.wilsonl.in/urls.txt
   - **Size**: ~16GB, hundreds of millions of URLs
   - **Origin**: Real search engine crawl
   - **Characteristics** (from sample analysis):
     - Average key length: ~47 bytes
     - Min: 3 bytes, Max: 151 bytes
     - Already lexicographically sorted
     - Very high prefix sharing (domains cluster together)
     - Domains: tumblr, dreamwidth, livejournal, etc.
   - **Estimated count**: ~340 million URLs (16GB / 47 bytes)
   - **Memory targets**:
     - BTreeMap naive: ~27GB (80 bytes/key)
     - Our target: ~3.4GB (10 bytes/key)
     - Stretch goal: ~1.7GB (5 bytes/key)

### Comparison Baselines

1. `std::collections::BTreeMap<String, V>`
2. `std::collections::HashMap<String, V>`
3. `fst` crate (for static comparison)
4. `radix_trie` crate

---

## 10. References & Papers

### Core Papers

1. **The Adaptive Radix Tree: ARTful Indexing for Main-Memory Databases**
   - Leis et al., 2013
   - Foundation for ART implementation
   - https://db.in.tum.de/~leis/papers/ART.pdf

2. **HOT: A Height Optimized Trie Index**
   - Binna et al., 2018
   - Improved ART variant
   - https://db.in.tum.de/~leis/papers/HOT.pdf

3. **SuRF: Practical Range Query Filtering**
   - Zhang et al., 2018
   - Succinct range filters

4. **The Case for Learned Index Structures**
   - Kraska et al., 2018
   - ML-based indexing (future consideration)

5. **FST: Fast Sequence Transducers**
   - BurntSushi's blog posts
   - Practical FST implementation details

### Rust Resources

1. `fst` crate documentation and source
2. `bumpalo` arena allocator
3. `parking_lot` for optimized locks

### Database Internals

1. RocksDB wiki on MemTable
2. LevelDB design doc
3. DuckDB internals documentation

---

## Appendix A: Memory Calculation Examples

### Example 1: 1 Billion Random 32-byte Keys

**BTreeMap<String, u64>**:
- String: 24 bytes + 32 bytes data = 56 bytes
- Value: 8 bytes
- BTree overhead: ~16 bytes/entry amortized
- Total: ~80 bytes/entry = 80 GB

**Our Target (Hybrid)**:
- Frozen layer: ~4 bytes/key (with prefix sharing)
- Delta layer: ~30 bytes/key (small percentage)
- Average: <10 bytes/key = <10 GB

### Example 2: 1 Billion Database Keys (prefix-heavy)

Keys like: `table:users:uuid:field`

**With prefix compression**:
- Common prefix "table:users:" shared
- UUID portion: 36 bytes
- Field portion: ~10 bytes average

**Our Target**:
- With path compression: ~6-8 bytes/key
- Total: ~7 GB

---

## Appendix B: ART Node Layouts

### Node4
```
┌─────────────────────────────────┐
│ type (1) │ count (1) │ prefix   │
├─────────────────────────────────┤
│ keys[4]  (4 bytes)              │
├─────────────────────────────────┤
│ children[4] (4×8 = 32 bytes)    │
└─────────────────────────────────┘
Total: ~48 bytes
```

### Node16
```
┌─────────────────────────────────┐
│ type (1) │ count (1) │ prefix   │
├─────────────────────────────────┤
│ keys[16] (16 bytes)             │
├─────────────────────────────────┤
│ children[16] (16×8 = 128 bytes) │
└─────────────────────────────────┘
Total: ~160 bytes
```

### Node48
```
┌─────────────────────────────────┐
│ type (1) │ count (1) │ prefix   │
├─────────────────────────────────┤
│ index[256] (256 bytes)          │
├─────────────────────────────────┤
│ children[48] (48×8 = 384 bytes) │
└─────────────────────────────────┘
Total: ~656 bytes
```

### Node256
```
┌─────────────────────────────────┐
│ type (1) │ prefix               │
├─────────────────────────────────┤
│ children[256] (256×8 = 2048)    │
└─────────────────────────────────┘
Total: ~2064 bytes
```

---

## Appendix C: Glossary

- **ART**: Adaptive Radix Tree
- **FST**: Finite State Transducer
- **Succinct**: Near-optimal space usage
- **LOUDS**: Level-Order Unary Degree Sequence
- **Patricia Trie**: Practical Algorithm to Retrieve Information Coded in Alphanumeric
- **Radix Tree**: Trie with edge compression
- **Arena**: Contiguous memory region for allocations
- **Compaction**: Merging delta layer into frozen layer

