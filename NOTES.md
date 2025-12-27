# hot-rs

Notes for a memory-efficient ordered map in Rust built around a Height Optimized Trie (HOT).

This file is written as the current-state description of the project (not a chronological log).

## Goal

Beat `BTreeMap<Vec<u8>, V>` memory usage on string-like byte keys while keeping practical performance for large, shuffled insert workloads.

## Result

`HotTree<()>` uses substantially less memory than `BTreeMap<Vec<u8>, ()>` on URL-style keys.

Snapshot (1,000,000 shuffled URLs):

```
Structure                  1M Keys   vs BTree
BTreeMap<Vec<u8>, ()>      117.0 MB   1.00x
HotTree<()>                 48.6 MB   2.41x
```

Memory breakdown at 1M keys (after `compact()` + `shrink_to_fit()`):
- Prefix pool: ~3.47 MB (shared prefix storage + hash table)
- Leaves: ~34.7 MB (prefix_id + suffix_len + suffix; no value_idx for ZST)
- Values: ~0 MB (ZST)
- Nodes: ~10.4 MB (HOT compound nodes + 5-byte child pointers)

Large-scale inserts have successfully completed on a 281,911,487‑URL dataset (15.9 GB total key bytes). Observed RSS depends heavily on allocator/system state; expect numbers on the order of ~20 GB for the full dataset, with leaf bytes dominating.

## Background

### Memory at scale

Trees dominated by per-node pointer overhead scale poorly:
- A binary node with two 64-bit pointers is already 16 bytes in pointers alone.
- Allocator metadata/fragmentation adds more overhead when nodes/keys are heap-allocated individually.

Tries are appealing for string-like keys because shared prefixes can be stored once.

### CPU-for-RAM trade

The design intentionally trades CPU and implementation complexity for fewer bytes per key:
- expensive bit extraction / SIMD is acceptable if it reduces memory,
- especially once the working set no longer fits in cache.

## Current Structure

`HotTree<V>` is an ordered map over `&[u8]` keys with:
- arena-based key storage (no per-key allocations),
- HOT compound nodes (2..=32 entries),
- packed 5‑byte child pointers,
- structural deletion.

### Key storage (prefix + suffix arenas)

Keys are stored in a contiguous leaf arena:

- `prefix_pool: Vec<u8>` stores distinct prefixes contiguously
- `prefix_offsets: Vec<u32>` stores the byte offset of each prefix in the pool (`prefix_id -> offset`)
- `prefix_hash: HashMap<u64, u16>` maps a prefix hash to `prefix_id` (with collision verification)

Each leaf encodes:
- `prefix_id: u16`
- `suffix_len: u8 | 0xFF + u16` (1 byte for `<255`, else 3 bytes total)
- `suffix_bytes`
- `value_idx: u32` (only when `V` is non‑ZST)

This eliminates allocator overhead for keys and makes key bytes the dominant term at scale (which is what we want; further wins are then “real compression”, not metadata).

### Key model limitations

At the bit level, missing bytes are treated as `0`. As a result, keys that differ only by trailing `0x00` bytes are not distinguishable. This is acceptable for typical “string-like” keys (URLs, file paths, etc.) but is a real limitation for arbitrary binary keys.

### Values

- For non‑ZST values, `values: Vec<Option<V>>` stores values and leaves store a `value_idx`.
- For ZST values, no `value_idx` is stored; a `Vec<V>` tracks live key count to preserve `Drop` semantics without allocating element bytes.

### Node arena + packed pointers

The topology is stored in `NodeArena`, a byte arena (`Vec<u8>`) with size‑class freelists for reuse.

Child pointers are stored as **40-bit tagged offsets** packed into **5 bytes**:
- bit 39 selects leaf arena vs node arena,
- bit 38 is reserved for a tombstone bit (currently unused by structural deletion),
- 38-bit offsets allow ~256 GiB of address space per arena.

This reduces per-child pointer cost from 8 bytes to 5 bytes.

## HOT Compound Nodes

HOT nodes combine multiple binary trie levels into a single “compound node” with up to 32 children.

Each HOT node stores:
- a discriminative-bits representation (mapping absolute key bit positions into a dense partial key),
- sparse partial keys (one per entry),
- child pointers.

### Discriminative-bits representations

Implemented mappings (from the HOT paper/reference), plus one extension for long keys:

- `SingleMask` (one 8-byte window)
- `MultiMask` (1 group, 8 bytes total)
- `MultiMask` (2 groups, 16 bytes total)
- `MultiMask` (4 groups, 32 bytes total)
- `MultiMask` (8 groups, 64 bytes total) — extension to support very long keys / wide byte spans

Sparse partial keys are stored as `u8`/`u16`/`u32` depending on the number of discriminative bits.

## Operations

### Lookup (`get`)

- Descends nodes by extracting a dense partial key and selecting the best matching sparse partial key (subset test).
- Uses AVX2 for the sparse partial key search when available (runtime detection); otherwise uses a scalar fallback.
- Uses BMI2 `pext` for extraction when available (runtime detection); otherwise uses a scalar fallback.

### Insert (`insert`)

- Descends to a leaf while recording the path.
- Computes the first differing bit against the leaf key.
- Rebuilds only the affected node/subtree (including full-node splitting) and integrates upward.

### Remove (`remove`)

Deletion is structural:
- removes the leaf entry from its parent by rebuilding the parent node (or collapsing to a sibling),
- updates ancestor child pointers in-place and fixes cached heights,
- frees replaced nodes into the arena freelists.

Removed keys are no longer reachable, but leaf bytes are append-only and are not reclaimed.

### Iteration (`iter`)

Depth-first traversal yields keys in order; keys are reconstructed into fresh `Vec<u8>` values from `prefix_id + suffix`.

### Compaction

- `compact()` rebuilds the node arena to remove fragmentation caused by node replacement during inserts/removes.
- `shrink_to_fit()` shrinks backing vectors; it can be expensive on very large trees because it may copy large allocations.

## Testing

Correctness testing is intentionally heavy because many failures are order-dependent:

- Unit tests for basic semantics and sorted iteration.
- Property tests (proptest) comparing against `BTreeMap<Vec<u8>, V>` under mixed `insert/remove/get/compact` sequences.
- Exhaustive permutation tests for small key sets (all insertion orders, all removal orders).
- Internal invariant checks:
  - node heights match children,
  - HOT node sparse partial keys remain monotonic,
  - reachable leaf count matches `len()`.

## Structures Considered

| Structure | Bytes/Key | Mutable | Notes |
|-----------|-----------|---------|-------|
| HOT | 11–14 | Yes | Bit-level branching, SIMD, complex |
| ART | 15–30 | Yes | Byte-aligned, used by DuckDB |
| FST | ~1.25 bits/node | No | LOUDS encoding, immutable |
| Judy | 5–7 | Yes | No good Rust impl |
| patricia_tree | ~32 | Yes | Production crate |

HOT’s headline advantage is low structural overhead (few bytes per key) by combining trie levels and branching on arbitrary bit positions.

## What worked (and is shipped)

- HOT compound nodes (2..=32 entries) reduce topology overhead versus pure binary nodes.
- Arena key storage removes allocator overhead and fragmentation from per-key allocations.
- Prefix deduplication captures the largest real redundancy in URL-like key sets.
- Packed pointers (5 bytes) reduce structural memory without giving up mutation.
- Architecture-specific fast paths (BMI2/AVX2) improve speed without changing the memory model.

## Approaches evaluated (not shipped)

These ideas were explored historically and are not part of the current implementation:

- ART-style nodes (4/16/48/256): good speed, but pointer-heavy; allocator overhead dominated memory at scale.
- Very small pointer schemes (u16 / u24 / u32 offsets): too little address space for multi‑GB key arenas.
- Sorted arrays / immutable succinct tries (FST/LOUDS): excellent memory, but not compatible with random-order mutation without costly rebuilds.
- Hybrid “mutable buffer + frozen layer”: feasible, but complexity was not justified versus continuing to compress the mutable structure.

## Techniques used

- Arena allocation: pack keys and nodes into contiguous vectors to avoid per-entry allocations.
- Prefix deduplication: store repeated prefixes once and reference them by id.
- Variable-length leaf header: 1–3 byte suffix length encoding.
- Packed pointers: 5-byte child pointers (40-bit tagged offsets).
- BMI2 `pext`: fast partial-key extraction when available.
- AVX2: fast sparse-partial-key descent when available.
- Structural deletion: avoids persistent tombstone overhead in steady-state lookups.

## Techniques considered (not currently used)

The best remaining memory wins are in key/leaf compression, because leaf bytes dominate:

- HOPE (order-preserving dictionary compression): can reduce key bytes but is complex to integrate with mutation.
- Front-coding / blocked delta encoding: strong compression for sorted inputs, but mutation complicates it.
- Leaf arena GC/compaction: reclaim key bytes after removals; requires relocations and pointer updates.

## Lessons (Current Takeaways)

- Per-node pointer size is the main structural cost lever; packing pointers is high value.
- Once topology is “small enough”, key bytes dominate and true compression becomes the priority.
- Bench speed varies significantly with CPU load and allocator state; memory ratios are more stable than raw ops/sec.
- Fast paths (BMI2/AVX2) help, but correctness invariants and testing are essential because HOT algorithms are subtle.

## Future Work (Memory-first)

- Key compression beyond prefix deduplication (front-coding blocks, shared suffix tables, or domain-specific URL schemes).
- Better reclamation for delete-heavy workloads (leaf compaction / GC).
- More compact freelist metadata to reduce overhead during massive incremental builds.

## References

Papers:
- ART (2013): “The Adaptive Radix Tree” — Leis, Kemper, Neumann
- HOT (2018): “HOT: A Height Optimized Trie Index for Main-Memory Database Systems” — Binna et al.
- HOPE (2020): “High-speed Order-Preserving Encoder” — SIGMOD 2020

Rust crates evaluated historically:
- `fst` (excellent memory, immutable)
- `patricia_tree`
- ART variants (`art-tree`, `rart`)
