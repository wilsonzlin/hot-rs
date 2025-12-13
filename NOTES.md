# hot-rs

Development notes for a memory-efficient ordered map in Rust.

## Goal

Beat `BTreeMap<Vec<u8>, V>` memory usage by 30%+ on string keys while maintaining >100K ops/sec.

## Result

`HotTree<V>`: **2.50x** less memory than BTreeMap on 1M URL keys.

```
Structure                  1M Keys   vs BTree
BTreeMap<Vec<u8>, ()>      117.0 MB   1.00x
HotTree<()>                 46.9 MB   2.50x
```

Memory breakdown at 1M keys:
- Prefix pool: 3.3 MB (shared prefix storage + hash table)
- Leaves: 33.1 MB (prefix_id + suffix_len + suffix + value_idx)
- Values: 1.0 MB (Option<()>)
- Nodes: 9.5 MB (1M binary nodes × 10 bytes in packed arena)

### Key Design Decisions

1. **Arena allocation for leaves**: Keys stored in contiguous `Vec<u8>` arena (no per-key allocation)
2. **Adaptive prefix compression**: Learns natural prefixes from delimiters (/, :, \)
3. **Packed node arena**: Binary nodes stored as raw bytes (10 bytes, no padding)
4. **Variable-length encoding**: suffix_len uses 1-3 bytes, value_idx uses 3 bytes
5. **32-bit tagged pointers**: Bit 31 distinguishes leaf vs node, bits 0-30 are offset
6. **ZST optimization**: No value_idx stored in leaves for zero-sized types
7. **Unsafe optimizations**: Direct pointer access for node read/write

### Optimization History

1. Baseline binary trie: 1.64x
2. Added prefix compression: 2.23x (+36%)
3. Hash-based prefix lookup: 2.45x (+10%)
4. Variable-length suffix_len: 2.50x (+2%)
5. Packed node arena (no padding): 2.50x (same ratio, faster)
6. 3-byte value_idx: 2.50x (already counted in leaves)

### Compaction Opportunities

Tree structure analysis shows:
- 327,855 nodes (33%) have both children as nodes
- These are potential N4 compound node candidates
- Estimated savings: ~2.5 MB if fully implemented

### What We Tried But Didn't Ship

**HOT compound nodes**: The paper uses nodes with 1-5 discriminator bits (2-32 children).
We implemented the arena structures but compaction is complex:
- Requires type tags (adds 1 byte per binary node)
- Need to handle mixed node types in same arena
- Post-processing compaction requires rebuilding entire tree

**8-byte nodes with 24-bit pointers**: Attempted to reduce node size by using
24-bit pointers (3 bytes each). Failed because 23-bit offset (8M max) cannot
address a 35 MB leaves arena.

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
    leaves: Vec<u8>,        // [len:2][key][value_idx:4]... (no value_idx for ZST)
    values: Vec<Option<V>>,
    nodes: Vec<BinaryNode>, // Fixed 10-byte nodes
    root: Ptr,              // 32-bit tagged pointer
    count: usize,
}

#[repr(C, packed)]
struct BinaryNode {
    disc: u16,   // discriminator bit position
    left: u32,   // child pointer for bit=0
    right: u32,  // child pointer for bit=1
}
```

BinaryNode: 10 bytes. `disc` (2) + `left` (4) + `right` (4).

Key advantages over previous approaches:
- Fixed-size nodes allow in-place child updates (no orphaning)
- Arena leaves eliminate per-key allocation overhead (~24 bytes/key saved)
- 32-bit pointers sufficient for 4GB address space (~80M keys)

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

---

## Phase 2: The Quest for 2x (Hot5-Hot11)

### New Goal

After achieving 1.33x improvement with HotTree, the question became: can we reach **2x**?

Target: Store 1M URLs in ~60MB instead of ~117MB.

### Hot5: Set Semantics Breakthrough

**Insight**: Many use cases don't need values—just set membership (contains/insert).

Removed value storage entirely:
```rust
pub struct Hot5 {
    key_data: Vec<u8>,    // [len:2][key bytes]... (no value_idx!)
    nodes: Vec<u8>,       // BiNodes: [bit_pos:2][left:4][right:4]
    root: Ptr,            // 32-bit tagged pointer
    count: usize,
}
```

Changes from HotTree:
- 32-bit pointers (4GB limit acceptable for sets)
- No value storage (saves 2 bytes/key + Option<V> overhead)
- BiNode: 10 bytes instead of 14

**Results on 1M URLs**:
```
BTreeMap<Vec<u8>, ()>:  117 MB  (baseline)
Hot5:                    73 MB  (1.60x improvement)
```

**Results across data types**:
```
Data Type      Hot5 Improvement
UUIDs          1.72x
File paths     1.91x
Hash strings   1.37x
Short keys     3.39x
```

Hot5 became the **recommended general-purpose** structure.

---

### Hot6-Hot7: Dead Ends

**Hot6: Full HOT compound nodes (again)**

Attempted proper HOT implementation with:
- Node types 1-5 (2, 4, 8, 16, 32 entries)
- Partial key extraction
- Dynamic node growth

Result: Still buggy. Complex bit manipulation for marginal gains. Abandoned.

**Hot7: Ultra-compact 8-byte nodes**

Tried 21-bit pointers for extreme compaction:
```rust
const BINODE_SIZE: usize = 8;  // bit_pos:2 + left:3 + right:3
```

Result: Panicked at 8MB. Address space exhausted. Only useful for tiny datasets.

---

### Hot8: First Prefix Deduplication Attempt

**Hypothesis**: URLs share prefixes (`https://`, `http://`, `s3://`). Store prefixes once.

```rust
const PREFIXES: [&[u8]; 8] = [
    b"", b"https://", b"http://", b"https://www.",
    b"http://www.", b"s3://", b"file://", b"ftp://",
];
```

Leaf format: `[prefix_id:1][suffix_len:2][suffix]`

**Bug #1: Off-by-one in prefix matching**
```rust
// Wrong:
if key.starts_with(prefix) && prefix.len() > best_len {
// Fixed:
if key.starts_with(prefix) && prefix.len() >= best_len {
```

**Bug #2: Missing prefix offset lookup**
```rust
// Wrong: Linear scan through prefix_pool
fn get_prefix(&self, id: u8) -> &[u8] {
    let mut off = 0;
    for _ in 0..id { /* scan */ }
}

// Fixed: O(1) lookup via prefix_offsets
fn get_prefix(&self, id: u8) -> &[u8] {
    let off = self.prefix_offsets[id as usize];
    // ...
}
```

**Bug #3: Empty prefix not registered**
```rust
// Wrong: Started with empty prefix_map
// Fixed:
fn new() -> Self {
    let mut s = Self { ... };
    s.get_or_create_prefix(b"");  // id=0 is empty prefix
    s
}
```

**Results**: Only 5-10% improvement. Static protocol prefixes don't capture domain sharing.

---

### Hot9: Domain Prefix Extraction

**Key insight**: URLs don't just share `https://`—they share `https://example.com/`.

Dynamic domain extraction:
```rust
fn extract_domain_prefix(key: &[u8]) -> &[u8] {
    // For "https://example.com/path" → "https://example.com/"
    if let Some(proto_end) = key.windows(3).position(|w| w == b"://") {
        let domain_start = proto_end + 3;
        if let Some(path_start) = key[domain_start..].iter().position(|&b| b == b'/') {
            return &key[..domain_start + path_start + 1];
        }
    }
    // ...
}
```

**Bug #4: Stack overflow on insert**
```rust
// Wrong: Recursive insert
fn insert_into(&mut self, ptr: Ptr, key: &[u8]) -> InsertResult {
    // ... recursive call
}

// Fixed: Iterative with parent tracking
fn insert(&mut self, key: &[u8]) -> bool {
    let mut path: Vec<(u32, bool)> = Vec::with_capacity(64);
    let mut current = self.root;
    loop {
        // ... iterative traversal
    }
}
```

**Bug #5: URLs without protocol**

Test data had bare hostnames like `example.com/path` (no `https://`).

```rust
// Wrong: Only looked for "://"
// Fixed:
fn extract_domain_prefix(key: &[u8]) -> &[u8] {
    if let Some(proto_end) = key.windows(3).position(|w| w == b"://") {
        // ... handle protocol URLs
    }
    // No protocol - find first '/' for bare domain URLs
    if let Some(path_start) = key.iter().position(|&b| b == b'/') {
        return &key[..path_start + 1];
    }
    key
}
```

**Results on URLs**:
```
BTreeMap:  117 MB
Hot9:       60 MB  (1.94x improvement!)
```

**Results on non-URLs**: CATASTROPHIC

```
Data Type      Hot9      Hot5     Verdict
URLs           1.94x     1.60x    Hot9 wins
UUIDs          0.91x     1.72x    Hot9 LOSES (worse than baseline!)
Random bytes   0.62x     1.37x    Hot9 LOSES badly
```

**Lesson**: Hot9 was overfit to URL structure.

---

### The Generalization Crisis

User feedback: **"Did you overfit to our dataset? You must create a general-purpose data structure."**

This was the pivotal moment. Hot9 achieved the 2x goal but only on URLs. We needed to understand *why* it worked and generalize.

**Analysis**:
1. URL domain prefixes are just prefixes ending at a delimiter (`/`)
2. File paths have similar structure (`/usr/local/bin/`)
3. S3 keys have similar structure (`s3://bucket/prefix/`)
4. The common pattern: **prefixes up to a natural delimiter**

---

### Hot11: Adaptive Delimiter Learning

**Design goals**:
1. Learn prefixes from data (no hardcoded patterns)
2. Use natural delimiters (`/`, `:`) as prefix boundaries
3. Graceful degradation on non-delimiter data
4. Give up if no structure detected

**Core algorithm**:
```rust
fn extract_natural_prefix(key: &[u8]) -> &[u8] {
    if key.len() < MIN_PREFIX_LEN { return &[]; }  // 8 byte minimum

    for i in MIN_PREFIX_LEN..key.len().min(MAX_PREFIX_LEN) {  // Up to 64
        let b = key[i];
        if b == b'/' || b == b':' {
            return &key[..=i];  // Include delimiter
        }
    }
    &[]
}
```

**Bug #6: Slow performance (83 seconds for 1M keys)**

Initial implementation checked every prefix in pool:
```rust
// Wrong: O(n) prefix matching
fn find_best_prefix(&self, key: &[u8]) -> (u16, usize) {
    for (prefix, &id) in &self.prefix_map {
        if key.starts_with(prefix) {
            // ...
        }
    }
}

// Fixed: O(1) - only check the natural prefix
fn find_best_prefix(&self, key: &[u8]) -> (u16, usize) {
    let natural = Self::extract_natural_prefix(key);
    if !natural.is_empty() {
        if let Some(&id) = self.prefix_map.get(natural) {
            return (id, natural.len());
        }
    }
    (0, 0)
}
```

Build time: 83s → 1.54s

**Bug #7: PROMOTION_THRESHOLD too high**

With threshold=4, many prefixes never got promoted:
```
Prefixes learned: 847
Expected on URL data: ~10,000 unique domains
```

Changed from 4 → 2 → 1 (always extract):
```rust
const PROMOTION_THRESHOLD: usize = 1;  // Always extract on first sight
```

**Bug #8: Give-up logic**

Without limits, Hot11 would track millions of candidates forever:
```rust
const LEARNING_GIVE_UP: usize = 1000;

fn maybe_learn_prefix(&mut self, key: &[u8]) {
    if !self.learning_enabled { return; }

    let natural = Self::extract_natural_prefix(key);
    if natural.is_empty() {
        self.inserts_since_promotion += 1;
        if self.inserts_since_promotion > LEARNING_GIVE_UP {
            self.learning_enabled = false;
            self.candidates.clear();
            self.candidates.shrink_to_fit();  // Free memory
        }
        return;
    }
    // ...
}
```

**Bug #9: Test overflow**
```rust
// Wrong:
let key = format!("{:08x}", i * 0x9e3779b9);  // Overflow!

// Fixed:
let key = format!("{:08x}", (i as u64).wrapping_mul(0x9e3779b9));
```

**Final Hot11 Results**:
```
Data Type      Hot11     Hot5     Hot9     Notes
URLs           1.86x     1.60x    1.94x    Close to Hot9
File paths     ~1.8x     1.91x    -        Works naturally
S3 keys        ~1.8x     1.60x    -        Works naturally
UUIDs          1.67x     1.72x    0.91x    Graceful (2% overhead)
Random         1.33x     1.37x    0.62x    Graceful (3% overhead)
```

---

### Variable-Length Headers

**Optimization**: Most prefix IDs < 255, save a byte:
```rust
fn store_leaf(&mut self, key: &[u8]) -> u32 {
    // ...
    if prefix_id < 255 {
        self.leaf_data.push(prefix_id as u8);        // 1 byte
    } else {
        self.leaf_data.push(255);                     // Marker
        self.leaf_data.extend_from_slice(&prefix_id.to_le_bytes()); // 2 bytes
    }
    self.leaf_data.extend_from_slice(&(suffix.len() as u16).to_le_bytes());
    self.leaf_data.extend_from_slice(suffix);
}
```

Header sizes:
- prefix_id < 255: 1 + 2 = 3 bytes
- prefix_id >= 255: 1 + 2 + 2 = 5 bytes

---

## HOPE Research Deep Dive

After Hot11, investigated related research.

### HOPE: High-speed Order-Preserving Encoder (SIGMOD 2020)

HOPE encodes keys using learned symbol dictionaries:

**N-gram schemes**:
```
Single-Char:  a→01, b→02, ...         ~1x compression
Double-Char:  th→A1, he→A2, ...       ~1.4x compression
3-Gram:       the→B1, ing→B2, ...     ~2x+ compression
```

**Key difference from hot-rs**:
- HOPE: Any repeated byte sequences, requires sampling/rebuild
- Hot11: Only prefixes at delimiters, fully online

**Comparison**:
```
Aspect              HOPE            Hot11
Pattern detection   N-gram freq     Delimiter-based
Target              Any repeated    Prefix only
Order preservation  Yes             No
Mutability          Rebuild needed  Full
Build cost          O(n) sampling   O(1) per insert
```

**Why we didn't use HOPE**:
1. Our constraint was **full mutability**
2. HOPE requires upfront key distribution sampling
3. Dictionary must be rebuilt when distribution changes
4. Marginal gains didn't justify complexity for our use case

**What we learned from HOPE**:
1. Delimiter-based prefixes capture most URL redundancy
2. More sophisticated encoding possible if mutability relaxed
3. Gap to theoretical minimum is ~24%

---

## Summary: Structure Recommendations

```
┌─────────────────────────────────────────────────────────────────────┐
│                    DECISION TREE                                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  Need mutability?                                                    │
│      │                                                               │
│      ├── No  → Use FST (fst crate, ~10 bits/key)                    │
│      │                                                               │
│      └── Yes → Do keys have delimiter structure?                    │
│                    │                                                 │
│                    ├── Unknown → Hot11 (adaptive, graceful)         │
│                    │                                                 │
│                    ├── Yes (URLs) → Hot9 (best for URLs)            │
│                    │                                                 │
│                    └── No (random) → Hot5 (general purpose)         │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Numbers Summary

### Memory Usage (1M URLs)
```
Structure              Memory      vs BTree    Build Time
BTreeMap               117 MB      1.00x       1.2s
HotTree                 88 MB      1.33x       1.8s
Hot5                    73 MB      1.60x       1.4s
Hot11                   63 MB      1.86x       1.5s
Hot9                    60 MB      1.94x       1.4s
Theoretical minimum     48 MB      2.44x       -
```

### Per-Key Overhead
```
Structure       Overhead/Key    Components
BTreeMap        72 bytes        Vec(24) + BTree node(48)
Hot5            25 bytes        key_len(2) + BiNode share(23)
Hot11           15 bytes        prefix_id(1) + suffix_len(2) + BiNode share(12)
Hot9            12 bytes        prefix_id(2) + suffix_len(2) + BiNode share(8)
```

### Breakdown of Hot11 on URLs
```
Component               Size        % of Total
Prefix pool             1.2 MB      1.9%
Prefix offsets          40 KB       0.1%
Prefix map overhead     400 KB      0.6%
Leaf data              42 MB       66.7%
Nodes                  19 MB       30.2%
Candidates (cleared)    0 MB        0%
─────────────────────────────────────────
Total                  62.6 MB     100%
```

---

## Lessons Learned (Phase 2)

### Technical
1. **Overfitting is real**: Hot9's URL-specific logic was 3x worse on random data
2. **Delimiters are universal**: `/` and `:` appear in URLs, paths, S3 keys, URIs
3. **Give-up heuristics matter**: Don't waste memory tracking useless candidates
4. **O(1) beats O(n)**: Prefix lookup must be constant-time for insert performance
5. **Variable headers work**: 1-byte vs 3-byte prefix_id saves significant space

### Process
1. **Test on diverse data**: One dataset isn't enough
2. **Measure overhead on worst case**: Random data reveals true baseline cost
3. **Generalization > optimization**: A 1.86x that works everywhere beats 1.94x that fails
4. **Read the research**: HOPE validated our direction, showed what's possible

### Design
1. **Adaptive > static**: Learning from data beats hardcoded patterns
2. **Graceful degradation**: Hot11's 2% overhead on random data is acceptable
3. **Simple structures compound**: Hot5's 10-byte BiNode × million keys matters
4. **Mutability has a cost**: ~24% gap to FST is the price of dynamic insertion

---

## Future Work

### Short-term
- [ ] SIMD prefix pool lookup
- [ ] Configurable delimiter set
- [ ] Prefix pool compaction (remove unused prefixes)

### Medium-term
- [ ] Hybrid Hot11 + front-coding for sorted bulk inserts
- [ ] Iterator support for Hot5/Hot9/Hot11
- [ ] Delete support (currently append-only)

### Research
- [ ] Learned prefix prediction (tiny neural net?)
- [ ] Hierarchical prefixes (`s3://bucket/` + `bucket/subdir/`)
- [ ] HOPE-style encoding for semi-static datasets

---

## Appendix: All Structures

| Name | Innovation | Status | Best For |
|------|-----------|--------|----------|
| HotTree | Original binary trie | Stable | Key-value maps |
| Hot2 | Compound nodes | Superseded | - |
| Hot3 | Burst containers | Superseded | - |
| Hot4 | 10-byte nodes | Superseded | - |
| **Hot5** | Set semantics | **Recommended** | General sets |
| Hot6 | HOT compound | Superseded | - |
| Hot7 | 8-byte nodes | Limited | Tiny datasets |
| Hot8 | Static prefixes | Superseded | - |
| **Hot9** | Domain extraction | **Specialized** | URL-only sets |
| Hot10 | (skipped) | - | - |
| **Hot11** | Adaptive learning | **Recommended** | Delimiter-structured |

---

---

## Phase 3: The 1.9x Breakthrough

### Starting Point

After Phase 2, the stable baseline was **1.64x** using sparse bitmap-indexed nodes with arena allocation. The core issue: any modification to variable-sized arena nodes creates orphans (wasted space).

### Approach 1: Sparse Nodes with Growth (FAILED)

**Hypothesis**: HOT compound nodes that grow by adding discriminators should be more efficient.

**Implementation**:
- Sparse bitmap-indexed nodes: `[num_disc:1][bitmap:4][discriminators:2*k][children:4*popcount]`
- Node growth: Add discriminators instead of creating new nodes
- Track full path to update grandparent when growing non-root nodes

**Bug encountered**: When growing a non-root parent node, incorrectly set `self.root = new_node` instead of updating grandparent.

**Fix**: Changed from single `parent` tracking to full `path: Vec<(u32, usize, u32, usize)>`.

**Result**: Tests passed but **1.34x** (worse than 1.64x baseline).

**Why it failed**: Every node modification (growth, child insertion) allocates a new node in the arena. Old nodes become orphaned garbage. The orphan overhead exceeded the savings from compound nodes.

### Approach 2: Sparse Nodes WITHOUT Growth (NO IMPROVEMENT)

**Hypothesis**: Maybe growth is the problem. Try sparse nodes with simple binary splits.

**Change**: Disabled node growth with `if false && p_num_disc < MAX_DISCRIMINATORS`.

**Result**: **1.64x** - same as baseline. The "empty slot" insertion path still created orphans by allocating new nodes with additional children.

**Lesson**: With variable-sized arena allocation, ANY modification orphans nodes. The sparse representation doesn't help if we keep creating new nodes.

### Approach 3: Fixed-Size Nodes with Vec<u8> Keys (FAILED)

**Hypothesis**: Use Rust's allocator for keys (Vec<u8> per leaf) but fixed-size nodes.

**Implementation**:
```rust
struct Leaf {
    key: Vec<u8>,
    value_idx: u32,
}
struct BinaryNode {
    disc: u16,
    left: Ptr,
    right: Ptr,
}
```

**Result**: Tests passed but **1.25x** (much worse).

**Why it failed**: Each `Vec<u8>` has 24-byte overhead (pointer + capacity + length). With 1M keys, that's 24MB of overhead just for key storage metadata.

**Lesson**: Per-key allocations are expensive. Arena allocation is critical for key storage.

### Approach 4: Arena Leaves + Fixed-Size Nodes (SUCCESS)

**Hypothesis**: Combine the best of both:
- Arena allocation for leaves (contiguous Vec<u8>)
- Fixed-size binary nodes in Vec<BinaryNode>

**Key insight**: Fixed-size nodes can be updated IN-PLACE. When we update a child pointer, we modify `nodes[idx].left = new_ptr` directly. No new allocation, no orphaning.

**Implementation**:
```rust
#[repr(C, packed)]
struct BinaryNode {
    disc: u16,   // 2 bytes
    left: u32,   // 4 bytes
    right: u32,  // 4 bytes
}  // Total: 10 bytes

leaves: Vec<u8>,        // Arena: [len:2][key][value_idx:4]
nodes: Vec<BinaryNode>, // Fixed-size, in-place updates
```

**Result**: **1.79x** at 500K keys!

### Optimization 1: shrink_to_fit()

**Problem**: At 1M keys, ratio dropped to 1.58x. Vec capacity doubling wasted space.

**Fix**: Added `shrink_to_fit()` method and called it after building.

**Result**: **1.80x** at both 500K and 1M keys. Consistent scaling.

### Optimization 2: ZST Value Optimization

**Observation**: Memory breakdown showed:
- Leaves: 54 MB
- Values: 1 MB (for Option<()>)
- Nodes: 9.5 MB

For zero-sized types like `()`, we don't need to store `value_idx` in leaves.

**Implementation**:
```rust
fn store_leaf(&mut self, key: &[u8]) -> u32 {
    // ...
    if std::mem::size_of::<V>() > 0 {
        let value_idx = self.values.len() as u32;
        self.leaves.extend_from_slice(&value_idx.to_le_bytes());
    }
    off
}
```

**Result**: **1.93x** at 1M keys, **1.91x** at 500K keys.

### Final Memory Breakdown (1M URLs)

```
Component    Size      Percentage
─────────────────────────────────
Leaves      50.23 MB   82.7%
Values       0.98 MB    1.6%
Nodes        9.54 MB   15.7%
─────────────────────────────────
Total       60.72 MB  100.0%
```

Theoretical minimum (raw key data): 48.33 MB
Actual overhead: 12.4 MB (25.6% over raw keys)

### Bug Fix: u16 value_idx Overflow

**Problem**: Original implementation used `u16` for value_idx, limiting to 65K values.

**Symptom**: Would silently wrap around at >65K keys, causing wrong values to be returned.

**Fix**: Changed to `u32` for value_idx (4 bytes instead of 2).

### Numbers Comparison

```
Approach                          500K Keys   1M Keys    Notes
──────────────────────────────────────────────────────────────
Baseline (sparse arena)            36 MB      74 MB     1.64x
Sparse with growth                 44 MB       -        1.34x (orphaning)
Sparse without growth              36 MB       -        1.64x (no gain)
Vec<u8> keys + fixed nodes         47 MB       -        1.25x (alloc overhead)
Arena leaves + fixed nodes         33 MB      74 MB     1.79x
  + shrink_to_fit                  33 MB      65 MB     1.80x
  + ZST optimization               31 MB      61 MB     1.91x
```

### Key Lessons

1. **Variable-size arena = orphan hell**: Any modification to variable-sized data in an arena creates orphans. Need either:
   - Fixed-size entries (our solution)
   - Free list / compacting GC (complex)
   - Accept the overhead

2. **In-place updates are gold**: Fixed-size `Vec<T>` entries can be modified without allocation. This eliminates orphaning for child pointer updates.

3. **Per-allocation overhead dominates**: `Vec<u8>` per key adds 24 bytes overhead. Arena allocation adds ~4 bytes (length field). 6x difference!

4. **shrink_to_fit matters**: Vec capacity doubling can waste 50% of allocated memory at scale.

5. **ZST optimization is free**: Compile-time `size_of::<V>() > 0` check costs nothing at runtime.

6. **Consistent scaling**: Good algorithms should have consistent ratios across data sizes. 1.80x → 1.58x was a red flag that led to the shrink_to_fit fix.

### What We Didn't Try (Future Work)

1. **Prefix compression**: Keys share prefixes. Could store `[common_prefix_id][suffix]` instead of full keys. Hot9/Hot11 from Phase 2 did this with ~1.94x results.

2. **Values Vec elimination for ZST**: Could skip the values Vec entirely for `()`. Would save ~1 MB at 1M keys.

3. **Variable-length key length field**: Most keys < 256 bytes. Could use 1 byte for short keys, 3 bytes for longer. Saves ~1 byte/key.

4. **Node pool with free list**: Reuse freed nodes instead of orphaning. Complex but would enable compound nodes.

---

## Architecture Evolution

```
Phase 1: HotTree (1.33x)
├── Arena everything
├── 48-bit pointers
└── Problem: Complex, marginal gains

Phase 2: Hot5-Hot11 (1.60x-1.94x)
├── Set semantics (Hot5)
├── Prefix compression (Hot9, Hot11)
└── Problem: Hot9 overfit to URLs

Phase 3: Unified HotTree (1.9x)
├── Arena leaves (contiguous keys)
├── Fixed-size binary nodes (in-place updates)
├── ZST optimization
└── General-purpose, no overfitting
```

---

## Phase 4: Scaling to 282 Million URLs

### Starting Point

After Phase 3, HotTree achieved ~1.9x improvement on 1M URLs. The question became: **does it scale?**

Test dataset: 281.91 million URLs (15.2 GB raw key data).

### Bug #1: Leaf Arena Overflow at 1GB

**Symptom**: At 30M URLs (~86% through), tree depth exploded from 159 to 11,124.

**Root cause**: The `Ptr` structure used 30 bits for byte offsets, limiting to ~1GB (0x3FFF_FFFF bytes). When `leaves.len()` exceeded this, pointer values wrapped around, corrupting the tree.

**Debug output revealed**:
```
LEAF ARENA OVERFLOW: leaves.len() = 1073741842 exceeds max offset 1073741823
```

**Fix**: Changed from byte offsets to **leaf indices**:
```rust
// OLD: Ptr stored byte offset directly
fn store_leaf(&mut self, key: &[u8]) -> u32 {
    let off = self.leaves.len() as u32;  // OVERFLOW at 1GB!
    // ...
    off
}

// NEW: Ptr stores index into leaf_offsets array
leaf_offsets: Vec<u64>,  // Maps leaf index → byte offset (u64 for >4GB)

fn store_leaf(&mut self, key: &[u8]) -> u32 {
    let byte_offset = self.leaves.len() as u64;
    let leaf_idx = self.leaf_offsets.len() as u32;  // Up to 2 billion leaves
    self.leaf_offsets.push(byte_offset);
    // ...
    leaf_idx
}
```

**Result**: 30M URL test passed. Tree depth stayed at 165.

### Bug #2: Node Arena Overflow at 1GB

**Symptom**: At 37% of 282M URLs (107M entries), panic with:
```
index out of bounds: the len is 107379594 but the index is 1677800763
```

**Root cause**: The bad index `0x63FF_FFBB` accidentally had what was previously the COMPOUND_BIT (bit 30) set. With 107M nodes × 10 bytes = 1.07 GB, byte offsets exceeded the 30-bit limit.

**Fix**: Applied same indirection pattern for nodes:
```rust
node_offsets: Vec<u64>,  // Maps node index → byte offset

fn alloc_node(&mut self, disc: u16, left: Ptr, right: Ptr) -> u32 {
    let byte_off = self.nodes.alloc_bi(disc, left, right);
    let node_idx = self.node_offsets.len() as u32;  // Up to 2 billion nodes
    self.node_offsets.push(byte_off as u64);
    node_idx
}
```

### Compaction Disabled

The compaction feature (converting binary nodes to N4 compound nodes) relied on using bit 30 as a "compound node" flag. After the node index refactoring, this no longer worked. Compaction was disabled pending a redesign.

### Final Results: 282 Million URLs

```
╔════════════════════════════════════════════════════════════╗
║                      Summary                               ║
╠════════════════════════════════════════════════════════════╣
║  BTreeMap:        34531.91 MB                              ║
║  HotTree:         20095.30 MB  (1.72x)                     ║
╚════════════════════════════════════════════════════════════╝
```

**Build time**: 53 minutes for HotTree vs 17.5 minutes for BTreeMap

**Tree depth**: 207 (stable throughout build)

**Memory breakdown**:
- Prefix pool: 3.31 MB (65,535 unique prefixes)
- Leaves: 14,983.81 MB
- Values: 268.85 MB
- Nodes: 4,839.33 MB

**Lookup performance**: 10,000 lookups in 0.119s (84K lookups/sec)

### Why 1.72x Instead of 1.9x?

The improvement ratio decreased from 1.9x (1M keys) to 1.72x (282M keys). Analysis:

1. **Indirection overhead**: The `leaf_offsets` and `node_offsets` vectors add 8 bytes per entry. At 282M entries, that's ~4.5 GB of overhead.

2. **More nodes**: At massive scale, the trie has more internal structure. 282M leaves required 282M nodes (roughly 1:1 ratio).

3. **BTreeMap scales better than expected**: Rust's BTreeMap is highly optimized. At scale, its constant factors become less dominant.

### Scaling Limits

With 31-bit indices, HotTree can now handle:
- Up to **2 billion leaves** (2^31 = 2,147,483,648)
- Up to **2 billion nodes**
- Unlimited total byte storage (u64 offsets)

### Lessons Learned

1. **Test at scale**: Bugs that never appear at 1M entries can be catastrophic at 100M+.

2. **Pointer bit-packing is fragile**: Using high bits for flags + low bits for offsets works until you run out of bits.

3. **Indirection beats bit-packing at scale**: Index → offset arrays use more memory but handle unlimited data sizes.

4. **Build time matters**: 3x slower build time (53 min vs 17.5 min) may be unacceptable for some use cases. Bulk loading optimization would help.

5. **Compaction needs rethinking**: The compound node optimization requires a separate tagging mechanism, not bit-packed into pointers.

### Performance Characteristics

```
Dataset Size    Build Time    Memory vs BTreeMap    Depth
1M URLs         ~1.5s         1.90x                 ~165
30M URLs        ~2 min        ~1.80x                ~165
282M URLs       53 min        1.72x                 207
```

The ~14 GB memory savings at 282M URLs (34.5 GB → 20.1 GB) is significant for large-scale applications, even with the slower build time.

---

*Updated December 2024*