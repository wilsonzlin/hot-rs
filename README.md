# hot-rs

A memory-efficient ordered map for Rust using a Height Optimized Trie (HOT).

`HotTree` provides similar functionality to `BTreeMap<Vec<u8>, V>` but uses **40-70% less memory** for typical workloads with byte-string keys.

## Usage

```rust
use hot_rs::HotTree;

let mut tree: HotTree<u64> = HotTree::new();

// Insert
tree.insert(b"apple", 1);
tree.insert(b"banana", 2);

// Get
assert_eq!(tree.get(b"apple"), Some(&1));

// Update (returns old value)
assert_eq!(tree.insert(b"apple", 10), Some(1));

// Remove
assert_eq!(tree.remove(b"banana"), Some(2));

// Iterate
for (key, value) in tree.iter() {
    println!("{:?} => {}", key, value);
}
```

## Memory Efficiency

Benchmarks on URL datasets (shuffled random inserts):

| Scale | BTreeMap | HotTree | Improvement |
|-------|----------|---------|-------------|
| 1M URLs | 117 MB | 61 MB | **1.9x** |
| 282M URLs | 34.5 GB | 20.1 GB | **1.7x** |

The improvement scales well from small to very large datasets.

## API

```rust
impl<V> HotTree<V> {
    fn new() -> Self;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn memory_usage(&self) -> usize;
    fn shrink_to_fit(&mut self);
}

impl<V: Clone> HotTree<V> {
    fn insert(&mut self, key: &[u8], value: V) -> Option<V>;
    fn get(&self, key: &[u8]) -> Option<&V>;
    fn contains_key(&self, key: &[u8]) -> bool;
    fn remove(&mut self, key: &[u8]) -> Option<V>;
    fn iter(&self) -> Iter<'_, V>;
}
```

## How It Works

HotTree uses a binary PATRICIA trie where:

- **Internal nodes** split on individual bit positions (discriminators)
- **Keys** are stored in a contiguous arena with adaptive prefix compression
- **Prefix compression** automatically learns common prefixes from delimiters (/, :, etc.)
- **31-bit indices** support up to 2 billion entries

This trades some CPU time for significant memory savings. Lookups are O(k) where k is key length in bits. The structure is ideal for applications with large key sets that need to minimize RAM usage.

## Limitations

- Keys are `&[u8]` (byte slices), not generic
- Values require `Clone` for retrieval
- `remove()` tombstones entries but doesn't reclaim space
- Build time is ~3x slower than BTreeMap (53 min vs 17 min for 282M keys)

## License

MIT OR Apache-2.0
