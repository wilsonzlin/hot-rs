# memkv

Memory-efficient key-value storage for string keys, designed for billion-key datasets.

## Benchmark Results (9.5M URLs, 467 MB raw data)

| Implementation | Memory | Overhead/Key | Insert ops/s | Lookup ops/s |
|---------------|--------|--------------|--------------|--------------|
| **FrozenLayer (FST)** | 320 MB | **-16.2 bytes** | 721K | 3.3M |
| **FastArt** | **1,040 MB** | **63.0 bytes** | **4.9M** | **8.6M** |
| libart (C) | 1,123 MB | 72.2 bytes | 5.0M | 11.7M |
| BTreeMap | 1,145 MB | 74.5 bytes | 3.3M | 8.5M |
| ArenaArt | 1,449 MB | 108.1 bytes | 3.5M | 11.1M |
| art-tree | 1,961 MB | 164.5 bytes | 3.2M | 5.5M |
| blart | 3,286 MB | 310.3 bytes | 2.3M | 9.4M |
| art | 6,953 MB | 713.9 bytes | 785K | 2.1M |
| rart | 9,875 MB | 1,035.6 bytes | 797K | 3.6M |

**FST achieves COMPRESSION** (negative overhead) for immutable data.  
**FastArt beats libart (C)** with 13% less memory and 100% correctness.  
**FastArt is 16x better** than rart (the SIMD-optimized Rust ART)!

## Usage

### FastArt (Mutable - Best Performance)

```rust
use memkv::FastArt;

let mut art = FastArt::new();
art.insert(b"key1", 1);
art.insert(b"key2", 2);

assert_eq!(art.get(b"key1"), Some(1));
assert_eq!(art.get(b"key2"), Some(2));
```

### FrozenLayer (Immutable - Best Memory)

```rust
use memkv::{FrozenLayerBuilder, FrozenLayer};

// Build from sorted keys
let mut builder = FrozenLayerBuilder::new()?;
builder.insert(b"key1", 1)?;
builder.insert(b"key2", 2)?;
let frozen: FrozenLayer = builder.finish()?;

// Query
assert_eq!(frozen.get(b"key1"), Some(1));
```

### ArenaArt (Mutable Alternative)

```rust
use memkv::ArenaArt;

let mut art = ArenaArt::new();
art.insert(b"key1", 1);
art.insert(b"key2", 2);

assert_eq!(art.get(b"key1"), Some(&1));
```

## Key Findings

1. **FST is unbeatable for immutable data** - provides 2.4x compression
2. **FastArt beats libart (C)** - 63 vs 72 bytes overhead, pure Rust, 100% correct
3. **BTreeMap is a solid baseline** - 75 bytes overhead, stdlib optimized
4. **Fixed-key-size ART crates are disasters** - rart uses 1036 bytes per key!

## Why Other Rust ARTs Fail

Most Rust ART crates (rart, blart, art) use **fixed-size key arrays**:

```rust
// rart forces 256-byte keys regardless of actual length!
let key: ArrayKey<256> = "example.com".into();  // 11 bytes → 256 bytes
```

For URLs averaging 51.5 bytes, this wastes **204 bytes per key**!

| Crate | Overhead | vs FastArt |
|-------|----------|------------|
| FastArt | 63 b | 1.0x |
| art-tree | 165 b | 2.6x |
| blart | 310 b | 4.9x |
| art | 714 b | 11.3x |
| rart | 1,036 b | **16.4x** |

## FastArt Optimizations

Inspired by libart (C), FastArt achieves excellent memory efficiency through:

1. **Pointer tagging** - Low bit distinguishes leaf vs internal node
2. **Inline key storage** - Keys embedded directly after leaf header
3. **Compact 16-byte headers** - Matching libart's proven design
4. **Terminating byte** - Handles prefix keys correctly
5. **Raw allocation** - Uses `std::alloc` for minimal overhead

## Recommendations

- **Read-heavy/frozen data**: Use `FrozenLayer`
- **Mutable data**: Use `FastArt`
- **Simple/portable**: Use `BTreeMap` 
- **Hybrid workloads**: FST base + FastArt delta layer

## License

MIT
