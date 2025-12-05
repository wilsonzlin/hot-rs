# MemKV - Memory-Efficient Key-Value Store

A Rust library designed for storing billions of string keys with extreme memory efficiency.

## Features

- **Adaptive Radix Tree (ART)**: Space-efficient trie with adaptive node sizes
- **Arena-Backed Storage**: Keys and prefixes stored in contiguous memory with 6-byte references
- **49% Memory Reduction**: Uses only 68 bytes/key vs 133 bytes/key for BTreeMap
- **Concurrent Access**: Thread-safe with RwLock
- **Rich Query API**: Single key lookup, range queries, prefix scans

## Performance (967K URL Dataset, 49 MB raw data)

| Implementation | Memory | Bytes/Key | vs BTreeMap |
|---------------|--------|-----------|-------------|
| std::BTreeMap | 122 MB | 133 | baseline |
| **UltraCompactArt** | **62 MB** | **68** | **-49%** |

**UltraCompactArt achieves 49% memory reduction** vs BTreeMap while maintaining 100% correctness.

## Usage

```rust
use memkv::UltraCompactArt;

let mut kv: UltraCompactArt<u64> = UltraCompactArt::new();

// Insert key-value pairs
kv.insert(b"user:1001", 42);
kv.insert(b"user:1002", 43);

// Lookup
assert_eq!(kv.get(b"user:1001"), Some(&42));

// Prefix scan
let users = kv.prefix_scan(b"user:");
```

Or use the thread-safe `MemKV` wrapper:

```rust
use memkv::MemKV;

let kv: MemKV<u64> = MemKV::new();
kv.insert(b"key", 42);
assert_eq!(kv.get(b"key"), Some(42));
```

## Architecture

The library provides three ART implementations:

### UltraCompactArt (Recommended)
- **Arena-backed keys AND prefixes**: All strings stored in a single contiguous arena
- **6-byte DataRef**: Instead of 24-byte `Vec<u8>`, uses packed `(u32 offset, u16 len)`
- **Best memory efficiency**: 68 bytes/key (49% less than BTreeMap)

### CompactArt
- **Arena-backed keys only**: Keys in arena, prefixes as `Vec<u8>`
- **Good balance**: 84 bytes/key (37% less than BTreeMap)

### AdaptiveRadixTree (Original)
- **Standard ART**: Traditional boxed node approach
- **Compatible baseline**: 145 bytes/key

All implementations use adaptive node sizing:
- **Node4**: 1-4 children (most common, smallest footprint)
- **Node16**: 5-16 children (sorted keys)
- **Node48**: 17-48 children (256-byte index + pointers)
- **Node256**: 49-256 children (direct array indexing)

## Memory Optimization Techniques

1. **Arena Allocation**: Keys and prefixes stored in contiguous memory
2. **Packed References**: 6-byte `DataRef` vs 24-byte `Vec<u8>` (saves 18 bytes per string)
3. **Boxing Large Arrays**: Node256's children array on heap to reduce enum size
4. **Prefix Compression**: Common prefixes stored once in tree structure

## Project Structure

```
memkv/
├── src/
│   ├── lib.rs           # Public API
│   ├── art/             # Original ART implementation
│   ├── art_compact/     # Arena-backed keys
│   ├── art_compact2/    # Arena-backed keys + prefixes (best)
│   ├── arena/           # Arena allocator
│   └── simple.rs        # BTreeMap-based fallback
├── examples/
│   ├── compare_memory.rs   # Memory comparison benchmark
│   └── url_dataset.rs      # URL dataset benchmark
└── benches/             # Criterion benchmarks
```

## Documentation

- [DESIGN.md](DESIGN.md) - Detailed design document with research
- [SCRATCHPAD.md](SCRATCHPAD.md) - Development notes and progress

## Future Work

- [ ] Child pointer compression (32-bit offsets in arena)
- [ ] FST integration for frozen/immutable data
- [ ] SIMD optimizations for Node16 lookup
- [ ] Concurrent writes with epoch-based reclamation

## License

MIT OR Apache-2.0
