# memkv

Memory-efficient key-value storage for string keys, designed for billion-key datasets.

## Benchmark Results (9.5M URLs, 467 MB raw data)

| Implementation | Memory | Overhead/Key | Insert ops/s | Lookup ops/s |
|---------------|--------|--------------|--------------|--------------|
| **FrozenLayer (FST)** | 320 MB | **-16.2 bytes** | 661K | 2.6M |
| **FastArt** | **998 MB** | **58.4 bytes** | **5.1M** | **9.9M** |
| libart (C) | 1,123 MB | 72.2 bytes | 4.9M | 9.6M |
| BTreeMap | 1,145 MB | 74.6 bytes | 3.3M | 7.8M |
| ArenaArt | 1,449 MB | 108.1 bytes | 3.1M | 10.7M |
| art-tree | 1,961 MB | 164.4 bytes | 3.8M | - |
| rart | 9,875 MB | 1,035.6 bytes | 1.6M | - |

**FST achieves COMPRESSION** (negative overhead) for immutable data.  
**FastArt beats libart (C)** with 19% less memory and 100% correctness.

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
2. **FastArt beats libart (C)** - 58 vs 72 bytes overhead, pure Rust
3. **BTreeMap is a solid baseline** - 75 bytes overhead, stdlib optimized
4. **Fixed-key-size ART crates are disasters** - rart uses 1035 bytes per key!

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
