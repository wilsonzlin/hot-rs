# memkv

Memory-efficient key-value storage for string keys, designed as a **drop-in BTreeMap replacement** with better memory efficiency.

## Quick Start (BTreeMap Replacement)

```rust
use memkv::FastArt;

// Create a map (replaces BTreeMap<Vec<u8>, u64>)
let mut map = FastArt::new();

// Insert keys in any order (random inserts supported!)
map.insert(b"user:12345", 1);
map.insert(b"session:abc", 2);
map.insert(b"cache:item:42", 3);

// Lookup
assert_eq!(map.get(b"user:12345"), Some(1));
assert_eq!(map.get(b"missing"), None);

// Update
map.insert(b"user:12345", 100);
assert_eq!(map.get(b"user:12345"), Some(100));
```

## Benchmark Results (1M Random Keys, avg 24 bytes)

| Structure | Memory Overhead | Insert/s | Lookup/s | vs BTreeMap |
|-----------|----------------|----------|----------|-------------|
| **ProperHot** | **+39 B/K** | 1.5M | 3.0M | **44% less memory** |
| **FastArt** | **+55 B/K** | **5.4M** | **8.9M** | **22% less, 2-3x faster** |
| BTreeMap | +71 B/K | 2.7M | 3.5M | (baseline) |

## When to Use What

| Use Case | Recommended | Why |
|----------|-------------|-----|
| High throughput (u64 values) | `FastArt` | 22% less memory, 2-3x faster |
| Lowest memory (u64 values) | `ProperHot` | 44% less memory |
| Generic values (`V`) | `MemKV<V>` | Flexible, thread-safe |
| Read-only/frozen data | `FrozenLayer` | Extreme compression |

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
