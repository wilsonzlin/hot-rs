# memkv

Memory-efficient key-value storage achieving **12 bytes/key overhead** - meeting the HOT paper target (10-14 B/K).

## Quick Start

```rust
// For minimum memory (12 B/K overhead):
use memkv::InlineHot;
let mut map = InlineHot::new();
map.insert(b"user:12345", 1);
assert_eq!(map.get(b"user:12345"), Some(1));

// For maximum speed:
use memkv::FastArt;
let mut map = FastArt::new();
map.insert(b"user:12345", 1);
```

## Benchmark Results (100K Random Keys, avg 23 bytes)

| Structure | Overhead | vs BTreeMap | Best For |
|-----------|----------|-------------|----------|
| **InlineHot** | **12 B/K** | **77% less memory** | Minimum memory |
| HOT | 16 B/K | 70% less | Good balance |
| FastArt | ~34 B/K | 37% less, 4x faster | Speed |
| BTreeMap | ~54 B/K | baseline | Compatibility |

*Overhead excludes raw key bytes and u64 values*

## When to Use What

| Use Case | Recommended | Why |
|----------|-------------|-----|
| Minimum memory | `InlineHot` | **12 B/K** - HOT paper target |
| Maximum speed | `FastArt` | 4x faster than BTreeMap |
| Generic values | `MemKV<V>` | Flexible API |
| Frozen data | `FrozenLayer` | Extreme compression |

## API Reference

### FastArt (Recommended for u64 values)

```rust
use memkv::FastArt;

let mut art = FastArt::new();
art.insert(b"key", 42);           // Insert
let v = art.get(b"key");          // Lookup: Some(42)
let old = art.insert(b"key", 99); // Update: returns Some(42)
let exists = art.get(b"key").is_some(); // Check existence
let count = art.len();            // Number of keys
```

### ProperHot (Best Memory Efficiency)

```rust
use memkv::ProperHot;

let mut hot = ProperHot::new();
hot.insert(b"key", 42);
assert_eq!(hot.get(b"key"), Some(42));
```

### FrozenLayer (Immutable, Compressed)

```rust
use memkv::FrozenLayer;

// Must insert in sorted order
let data = vec![
    (b"apple".as_slice(), 1u64),
    (b"banana".as_slice(), 2u64),
    (b"cherry".as_slice(), 3u64),
];

let frozen = FrozenLayer::from_sorted_iter(data).unwrap();
assert_eq!(frozen.get(b"apple"), Some(1));
```

### MemKV<V> (Generic, Thread-Safe)

```rust
use memkv::MemKV;

let map: MemKV<String> = MemKV::new();
map.insert(b"key", "value".to_string());
assert_eq!(map.get(b"key"), Some("value".to_string()));
```

## Implementation Details

FastArt is an Adaptive Radix Tree (ART) with:

- **O(key_length)** operations (vs O(log n) for BTreeMap)
- **SIMD-optimized** Node16 child lookup (SSE2)
- **Pointer tagging** to distinguish leaves from internal nodes
- **Path compression** to reduce tree height
- **Compact node layouts** matching libart (C) design

## Large Scale Benchmarks (9.5M URLs)

| Implementation | Memory | Overhead/Key | Insert ops/s |
|---------------|--------|--------------|--------------|
| **FrozenLayer (FST)** | 320 MB | **-16 bytes** | 721K |
| **FastArt** | 1,040 MB | 63 bytes | 4.9M |
| libart (C) | 1,123 MB | 72 bytes | 5.0M |
| BTreeMap | 1,145 MB | 75 bytes | 3.3M |

- **FrozenLayer** achieves compression (negative overhead) for immutable data
- **FastArt** beats libart (C) with 13% less memory

## License

MIT
