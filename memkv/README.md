# memkv

Memory-efficient key-value storage. 33% less memory than BTreeMap.

## Quick Start

```rust
use memkv::InlineHot;

let mut map = InlineHot::new();
map.insert(b"user:12345", 42u64);
assert_eq!(map.get(b"user:12345"), Some(42));
```

## Structures

| Structure | Memory | Speed | Use Case |
|-----------|--------|-------|----------|
| `InlineHot` | **-33%** | 2.1M/s | Minimum memory |
| `FastArt` | -6% | **5.2M/s** | Maximum speed |
| `FrozenLayer` | -40%* | 3.3M/s | Immutable data |
| `MemKV<V>` | -33% | 2M/s | Thread-safe, generic values |

*vs BTreeMap on 500K URLs benchmark

## Usage

### InlineHot (Best Memory)

```rust
use memkv::InlineHot;

let mut map = InlineHot::new();
map.insert(b"key", 1u64);
map.insert(b"key", 2u64);  // Update
assert_eq!(map.get(b"key"), Some(2));
println!("entries: {}", map.len());
```

### FastArt (Best Speed)

```rust
use memkv::FastArt;

let mut art = FastArt::new();
art.insert(b"key", 1u64);
assert_eq!(art.get(b"key"), Some(1));
```

### FrozenLayer (Immutable)

```rust
use memkv::FrozenLayer;

// Keys must be sorted
let data = vec![
    (b"a".as_slice(), 1u64),
    (b"b".as_slice(), 2u64),
];
let frozen = FrozenLayer::from_sorted_iter(data).unwrap();
assert_eq!(frozen.get(b"a"), Some(1));
```

### MemKV (Thread-safe)

```rust
use memkv::MemKV;

let kv: MemKV<String> = MemKV::new();
kv.insert(b"name", "Alice".to_string());
assert_eq!(kv.get(b"name"), Some("Alice".to_string()));

// Prefix scan
for (key, value) in kv.prefix(b"user:") {
    println!("{:?} = {}", key, value);
}
```

## Benchmarks

500K URLs, shuffled random inserts:

```
Structure      Total MB   Overhead B/K   Lookup/s
─────────────────────────────────────────────────
BTreeMap           52.0         57.7        2.1M
InlineHot          34.6         22.7        2.1M  ← -33% memory
FastArt            49.1         51.7        5.2M  ← 2.5x faster
```

Run yourself:
```bash
cargo run --release --example scale_test
```

## How It Works

**InlineHot** uses a Height Optimized Trie with:
- BiNodes (binary splits on discriminating bits)
- 48-bit pointers (saves 2 bytes vs 64-bit, supports up to 128TB)
- Inline value storage (no separate leaf struct)
- ~14 B/K index overhead
- Tested with 282M URLs (16GB raw data)

**FastArt** uses Adaptive Radix Tree with:
- Four node sizes (4, 16, 48, 256 children)
- SIMD search for Node16
- Pointer tagging for leaves
- Path compression

## License

MIT
