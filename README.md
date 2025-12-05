# MemKV - Memory-Efficient Key-Value Store

A Rust library designed for storing and querying billions of string keys with extreme memory efficiency.

## Status

**Phase 1 Complete** - Baseline implementation with BTreeMap backend for correctness.

### What Works
- ✅ Insert, get, remove operations
- ✅ Range queries
- ✅ Prefix scans
- ✅ Thread-safe (RwLock)
- ✅ 100% correctness verified against 100K URL dataset

### What's In Progress
- 🔧 ART (Adaptive Radix Tree) implementation - has bug, being debugged
- 🔧 FST (Finite State Transducer) frozen layer
- 🔧 Arena allocation for memory efficiency

## Quick Start

```rust
use memkv::MemKV;

// Create store
let kv: MemKV<u64> = MemKV::new();

// Insert keys
kv.insert(b"user:1001", 1001);
kv.insert(b"user:1002", 1002);

// Point lookup
assert_eq!(kv.get(b"user:1001"), Some(1001));

// Prefix scan
for (key, value) in kv.prefix(b"user:") {
    println!("{:?} -> {}", key, value);
}

// Range query
for (key, value) in kv.range(b"user:1000", b"user:2000") {
    println!("{:?} -> {}", key, value);
}
```

## Performance

Current implementation (BTreeMap backend):

| Metric | Value |
|--------|-------|
| Insert | ~5M ops/sec |
| Lookup | ~10M ops/sec |
| Memory | ~113 bytes/key |

Target (ART + FST):

| Metric | Target |
|--------|--------|
| Insert | >100K ops/sec |
| Lookup | >100K ops/sec |
| Memory | <10 bytes/key |

## Benchmarking

```bash
# Run with 100K synthetic keys
cargo run --release --example memory_test -- 100000

# Run with URL dataset
curl -r 0-10000000 "https://static.wilsonl.in/urls.txt" > urls_10mb.txt
cargo run --release --example url_dataset -- urls_10mb.txt

# Run criterion benchmarks
cargo bench
```

## Architecture

The library is designed with a hybrid architecture:

1. **Delta Layer (ART)** - Mutable, for recent writes
2. **Frozen Layer (FST)** - Immutable, highly compressed
3. **Background Compaction** - Merges delta into frozen

Currently, only the SimpleKV (BTreeMap-based) backend is active while the ART implementation is being debugged.

## Documentation

- [DESIGN.md](DESIGN.md) - Comprehensive design document
- [SCRATCHPAD.md](SCRATCHPAD.md) - Development notes and progress

## License

MIT OR Apache-2.0
