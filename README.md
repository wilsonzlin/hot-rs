# hot-rs

A memory-efficient ordered map for Rust using a Height Optimized Trie (HOT).

`HotTree` provides similar functionality to `BTreeMap<Vec<u8>, V>` but is optimized for **minimum RAM per key** on large sets of string-like byte keys (e.g. URLs).

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
| 1M URLs | 117 MB | 48.6 MB | **2.4x** |
| 282M URLs | 34.5 GB | ~19–20 GB | **~1.7–1.8x** |

The improvement scales well from small to very large datasets.

## API

```rust
impl<V> HotTree<V> {
    pub fn new() -> Self;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;

    pub fn insert(&mut self, key: &[u8], value: V) -> Option<V>;
    pub fn get(&self, key: &[u8]) -> Option<&V>;
    pub fn contains_key(&self, key: &[u8]) -> bool;
    pub fn remove(&mut self, key: &[u8]) -> Option<V>;
    pub fn iter(&self) -> Iter<'_, V>;

    pub fn memory_usage(&self) -> usize;
    pub fn shrink_to_fit(&mut self);
    pub fn compact(&mut self) -> usize;
}
```

## How It Works

HotTree uses a binary PATRICIA trie where:

- **Internal nodes** split on individual bit positions (discriminators), combined into HOT-style compound nodes (up to 32-way)
- **Keys** are stored in a contiguous arena with adaptive prefix compression
- **Prefix compression** automatically learns common prefixes from delimiters (/, :, etc.)
- **Node pointers** are stored as packed 40-bit tagged offsets (5 bytes per child pointer)

This trades some CPU time for significant memory savings. Lookups are O(k) where k is key length in bits. The structure is ideal for applications with large key sets that need to minimize RAM usage.

## Limitations

- Keys are `&[u8]` (byte slices), not generic
- Keys that differ only by trailing `0x00` bytes are not distinguishable (optimized for “string-like” keys)
- `remove()` does not reclaim leaf/key bytes in the append-only leaf arena
- `iter()` reconstructs keys into fresh `Vec<u8>` allocations

## License

MIT OR Apache-2.0
