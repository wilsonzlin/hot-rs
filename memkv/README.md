# memkv

Memory-efficient key-value storage for string keys, designed for billion-key datasets.

## Benchmark Results (9.5M URLs, 467 MB raw data)

| Implementation | Memory | Overhead/Key | Insert ops/s |
|---------------|--------|--------------|--------------|
| **FrozenLayer (FST)** | 320 MB | **-30.1 bytes** | 680K |
| BTreeMap | 1,145 MB | 74.6 bytes | 3.8M |
| **ArenaArt** | 1,449 MB | 108.1 bytes | 3.3M |
| art-tree | 1,961 MB | 164.4 bytes | 3.8M |
| blart | 3,286 MB | 310.3 bytes | 3.0M |
| rart | 9,875 MB | 1,035.6 bytes | 1.6M |

**FST achieves COMPRESSION** (negative overhead) for immutable data.  
**ArenaArt beats ALL existing Rust ART crates** in memory efficiency.

## Usage

### FrozenLayer (Immutable - Best Memory)

```rust
use memkv::{FrozenLayerBuilder, FrozenLayer};

// Build from sorted keys
let mut builder = FrozenLayerBuilder::new();
builder.insert(b"key1", 1)?;
builder.insert(b"key2", 2)?;
let frozen: FrozenLayer = builder.finish()?;

// Query
assert_eq!(frozen.get(b"key1"), Some(1));
```

### ArenaArt (Mutable)

```rust
use memkv::ArenaArt;

let mut art = ArenaArt::new();
art.insert(b"key1", 1);
art.insert(b"key2", 2);

assert_eq!(art.get(b"key1"), Some(&1));
```

## Key Findings

1. **FST is unbeatable for immutable data** - provides 2.4x compression
2. **BTreeMap beats ART for mutable data** - surprisingly efficient due to high fanout
3. **Fixed-key-size ART crates are disasters** - rart uses 1035 bytes per key!
4. **Our ArenaArt is the best Rust ART** - variable-length keys + arena allocation

## Recommendations

- **Read-heavy/frozen data**: Use `FrozenLayer`
- **Mutable data**: Use `BTreeMap` or `ArenaArt`
- **Hybrid workloads**: FST base + mutable delta layer

## License

MIT
