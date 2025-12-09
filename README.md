# hot-rs

A memory-efficient ordered map for Rust using a Height Optimized Trie (HOT).

`HotTree` provides similar functionality to `BTreeMap<Vec<u8>, V>` but uses **~33% less memory** for typical workloads with byte-string keys.

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

Benchmark with 500K URL keys (shuffled random inserts):

| Structure | Memory | Overhead/Key |
|-----------|--------|--------------|
| `BTreeMap<Vec<u8>, u64>` | 52 MB | 57.7 bytes |
| `HotTree<u64>` | 35 MB | 22.7 bytes |

Run the benchmark yourself:

```bash
cargo run --release --example benchmark
```

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
    fn range<R: RangeBounds<&[u8]>>(&self, range: R) -> Range<'_, V, R>;
}
```

## How It Works

HotTree uses a binary trie (BiNode structure) where:

- **Internal nodes** split on individual bit positions in the key
- **Keys and values** are stored inline in contiguous arenas
- **48-bit pointers** reduce per-pointer overhead while supporting up to 128TB

This trades some CPU time for significant memory savings. Lookups are O(k) where k is key length in bits. The structure is ideal for applications with large key sets that need to minimize RAM usage.

## Limitations

- Keys are `&[u8]` (byte slices), not generic
- Values require `Clone` for retrieval
- `remove()` tombstones entries but doesn't reclaim space (rebuild tree for compaction)
- `iter()` and `range()` traverse the full tree; O(n) not O(log n + k)

## License

MIT OR Apache-2.0
