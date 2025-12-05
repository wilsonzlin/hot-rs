# Design

## Goals

Store billions of string keys with minimal memory overhead while maintaining reasonable performance (~100K+ ops/sec).

## Approach

Two main data structures for different use cases:

1. **FastArt**: Mutable Adaptive Radix Tree for read-write workloads
2. **FrozenLayer**: Immutable FST for read-only data with maximum compression

## FastArt

Based on the [ART paper](https://db.in.tum.de/~leis/papers/ART.pdf) and [libart](https://github.com/armon/libart).

### Node Types

```
Node4:   16-byte header + 4 keys + 4 pointers = 48 bytes
Node16:  16-byte header + 16 keys + 16 pointers = 160 bytes
Node48:  16-byte header + 256-byte index + 48 pointers = 664 bytes
Node256: 16-byte header + 256 pointers = 2064 bytes
Leaf:    8-byte value + 4-byte key_len + key bytes
```

### Key Optimizations

1. **Pointer tagging**: Low bit distinguishes leaf vs internal node
2. **Inline keys**: Keys stored directly after leaf header (no separate allocation)
3. **Compact headers**: 16 bytes matching libart's proven design
4. **Terminating byte**: Handles prefix-of-another-key correctly
5. **Path compression**: Up to 10 bytes of prefix stored inline

### Memory Overhead

On URL-like data averaging 51 bytes per key:
- FastArt: ~63 bytes overhead per key
- BTreeMap: ~75 bytes overhead per key
- Other Rust ART crates: 165-1035 bytes overhead (fixed-size key arrays)

## FrozenLayer

Uses the `fst` crate for FST (Finite State Transducer) construction.

### Properties

- Shares prefixes AND suffixes (unlike ART which only shares prefixes)
- Achieves compression rather than overhead
- O(key_length) lookups
- Efficient range queries and prefix scans
- Requires sorted input during construction
- Values limited to u64

### Compression

On 467 MB of URL data:
- Raw: 467 MB
- FST: 194 MB (2.4x compression)
- Overhead: -16 bytes per key

## Trade-offs

| | FastArt | FrozenLayer |
|-|---------|-------------|
| Mutable | Yes | No |
| Memory | ~63 bytes/key | Negative (compression) |
| Lookup | O(key_len) | O(key_len) |
| Insert | O(key_len) | N/A |
| Range queries | No | Yes |
| Prefix scans | No | Yes |
| Construction | Incremental | Batch (sorted) |

## Why Other Rust ART Crates Fail

Most Rust ART implementations (rart, blart, art) use fixed-size key arrays:

```rust
// Forces 256 bytes per key regardless of actual length
let key: ArrayKey<256> = "example.com".into();  // 11 bytes → 256 bytes
```

For URLs averaging 51 bytes, this wastes 200+ bytes per key.

## Future Work

- Range queries for FastArt
- SIMD optimization for Node16 child lookup
- Concurrent FastArt with fine-grained locking
- Hybrid store: FST base + FastArt delta layer
