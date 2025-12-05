# memkv

Memory-efficient key-value storage for string keys.

## Results

On 9.5M URLs (467 MB raw key data):

| Implementation | Memory | Per-Key Overhead |
|----------------|--------|------------------|
| FrozenLayer    | 320 MB | -16 bytes (compression) |
| FastArt        | 1,040 MB | 63 bytes |
| BTreeMap       | 1,145 MB | 75 bytes |

## Usage

### FastArt (mutable)

```rust
use memkv::FastArt;

let mut art = FastArt::new();
art.insert(b"key1", 1);
art.insert(b"key2", 2);

assert_eq!(art.get(b"key1"), Some(1));
```

### FrozenLayer (immutable, best compression)

```rust
use memkv::FrozenLayer;

// Keys must be sorted
let data = vec![
    (b"apple".as_slice(), 1u64),
    (b"banana".as_slice(), 2u64),
];

let frozen = FrozenLayer::from_sorted_iter(data).unwrap();
assert_eq!(frozen.get(b"apple"), Some(1));
```

### MemKV (thread-safe wrapper)

```rust
use memkv::MemKV;

let kv = MemKV::new();
kv.insert(b"key", 42);
assert_eq!(kv.get(b"key"), Some(42));
```

## Implementations

**FastArt**: Adaptive Radix Tree inspired by [libart](https://github.com/armon/libart).
- Pointer tagging to distinguish leaf vs internal nodes
- Inline key storage in leaf allocations
- Adaptive node sizing (Node4/16/48/256)
- ~63 bytes overhead per key

**FrozenLayer**: FST (Finite State Transducer) via the `fst` crate.
- Shares both prefixes and suffixes
- Achieves compression (negative overhead)
- Requires sorted input, immutable after construction
- Supports range queries and prefix scans

**SimpleKV**: BTreeMap wrapper for comparison baseline.

## When to Use

| Use Case | Recommendation |
|----------|----------------|
| Read-only/frozen data | FrozenLayer |
| Read-write workloads | FastArt or MemKV |
| Need ordering/range queries on frozen data | FrozenLayer |
| Simple baseline | SimpleKV |

## License

MIT OR Apache-2.0
