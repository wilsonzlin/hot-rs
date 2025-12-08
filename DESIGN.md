# Memory-Efficient Key-Value Store

See `memkv/` for the implementation.

## Results

500K URLs, shuffled random inserts:

| Structure | Total Memory | vs BTreeMap | Lookup/s |
|-----------|-------------|-------------|----------|
| BTreeMap | 52.0 MB | baseline | 2.1M |
| **InlineHot** | **34.6 MB** | **-33%** | 2.1M |
| FastArt | 49.1 MB | -6% | **5.2M** |

## Documentation

- `memkv/README.md` - Usage guide
- `memkv/INTERNALS.md` - Engineering details, design decisions, lessons learned
- `memkv/docs/` - Research references

## Quick Start

```rust
use memkv::InlineHot;

let mut map = InlineHot::new();
map.insert(b"key", 42u64);
assert_eq!(map.get(b"key"), Some(42));
```
