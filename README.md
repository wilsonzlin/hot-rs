# MemKV - Memory-Efficient Key-Value Store

A Rust library designed for storing billions of string keys with extreme memory efficiency.

## Key Results (967K URL Dataset, 46 MB raw data)

| Implementation | Memory | Bytes/Key | vs BTreeMap |
|---------------|--------|-----------|-------------|
| **FrozenLayer (FST)** | **40 MB** | **44** | **-65% ✓** |
| std::BTreeMap | 115 MB | 125 | baseline |
| ArenaArt | 180 MB | 195 | +57% |
| UltraCompactArt | 192 MB | 208 | +67% |

**FrozenLayer achieves 65% memory reduction** vs BTreeMap for immutable data.

## Features

- **FrozenLayer (FST)**: Extreme compression for read-only data using Finite State Transducers
- **ArenaArt**: Mutable ART with arena-based node storage
- **Concurrent Access**: Thread-safe with RwLock wrapper
- **Rich Query API**: Single key lookup, range queries, prefix scans
- **Accurate Measurement**: jemalloc integration for precise memory tracking

## Quick Start

### For Read-Only/Frozen Data (Best Memory Efficiency)

```rust
use memkv::FrozenLayer;

// Keys must be sorted for FST construction
let mut data: Vec<(&[u8], u64)> = vec![
    (b"apple", 1),
    (b"banana", 2),
    (b"cherry", 3),
];
data.sort_by_key(|(k, _)| *k);

let frozen = FrozenLayer::from_sorted_iter(data).unwrap();

// Lookups
assert_eq!(frozen.get(b"apple"), Some(1));

// Prefix scans
let results = frozen.prefix_scan(b"a");

// Range queries
let results = frozen.range(b"a", b"c");
```

### For Mutable Data

```rust
use memkv::ArenaArt;

let mut kv: ArenaArt<u64> = ArenaArt::new();

kv.insert(b"user:1001", 42);
kv.insert(b"user:1002", 43);

assert_eq!(kv.get(b"user:1001"), Some(&42));
```

### Thread-Safe Wrapper

```rust
use memkv::MemKV;

let kv: MemKV<u64> = MemKV::new();
kv.insert(b"key", 42);
assert_eq!(kv.get(b"key"), Some(42));
```

## Architecture

### FrozenLayer (Recommended for read-only data)

Uses **Finite State Transducers (FST)** for extreme compression:
- ~25 bytes/key for URL dataset (2x compression vs raw)
- O(key length) lookups
- Efficient range queries and prefix scans
- Must provide sorted input during construction

### ArenaArt (For mutable data)

**Adaptive Radix Tree** with arena-based node storage:
- Nodes stored in contiguous Vec (no per-node allocation overhead)
- Keys and prefixes in data arena with 6-byte references
- Adaptive node sizing: Node4 → Node16 → Node48 → Node256

### Other Implementations

- **UltraCompactArt**: Box-based nodes with arena-backed strings
- **CompactArt**: Arena-backed keys only
- **AdaptiveRadixTree**: Traditional ART implementation
- **SimpleKV**: BTreeMap-based fallback

## When to Use What

| Use Case | Recommendation | Memory |
|----------|----------------|--------|
| Read-only data | FrozenLayer | ~44 bytes/key |
| Mutable + range queries | BTreeMap or ArenaArt | ~125-195 bytes/key |
| Very long keys + prefixes | ArenaArt | Good prefix compression |
| Hybrid workloads | FST base + mutable delta | Best of both |

## Memory Optimization Techniques

1. **FST Compression**: Finite State Transducers for immutable data
2. **Arena Allocation**: Nodes in contiguous Vec, strings in data arena
3. **Packed References**: 6-byte `DataRef` vs 24-byte `Vec<u8>`
4. **Boxing Large Arrays**: Reduces enum size to 56 bytes
5. **jemalloc**: Accurate allocation tracking via tikv-jemalloc-ctl

## Project Structure

```
memkv/
├── src/
│   ├── lib.rs           # Public API
│   ├── frozen/          # FST-based frozen layer (best!)
│   ├── art_arena/       # Arena-based ART
│   ├── art_compact2/    # Box-based ART with arena strings
│   ├── arena/           # Arena allocator
│   └── simple.rs        # BTreeMap fallback
├── examples/
│   ├── compare_all.rs   # Complete memory comparison
│   └── url_dataset.rs   # URL dataset benchmark
└── benches/             # Criterion benchmarks
```

## Benchmarking

```bash
# Run complete memory comparison
cargo run --release --example compare_all

# Run with jemalloc for accurate measurements
cargo run --release --example compare_all
```

## Documentation

- [DESIGN.md](DESIGN.md) - Detailed design document with research
- [SCRATCHPAD.md](SCRATCHPAD.md) - Development notes and progress

## Future Work

- [ ] Hybrid store combining FST + mutable delta
- [ ] Incremental FST updates
- [ ] SIMD optimizations for Node16 lookup
- [ ] Concurrent writes with epoch-based reclamation

## License

MIT OR Apache-2.0
