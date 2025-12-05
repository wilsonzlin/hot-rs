//! Basic usage examples for memkv.

use memkv::{FastArt, FrozenLayer, MemKV};

fn main() {
    example_memkv();
    example_fast_art();
    example_frozen_layer();
}

fn example_memkv() {
    println!("=== MemKV (Thread-Safe Wrapper) ===\n");

    let kv = MemKV::new();

    // Insert data
    kv.insert(b"user:1001", 1001);
    kv.insert(b"user:1002", 1002);
    kv.insert(b"user:1003", 1003);

    // Lookups
    println!("user:1001 = {:?}", kv.get(b"user:1001"));
    println!("user:9999 = {:?}", kv.get(b"user:9999"));
    println!("Contains user:1002: {}", kv.contains(b"user:1002"));
    println!("Count: {}\n", kv.len());
}

fn example_fast_art() {
    println!("=== FastArt (Mutable ART) ===\n");

    let mut art = FastArt::new();

    // Insert data
    art.insert(b"http://example.com/page1", 1);
    art.insert(b"http://example.com/page2", 2);
    art.insert(b"http://other.com/page1", 3);

    // Lookups
    println!("example.com/page1 = {:?}", art.get(b"http://example.com/page1"));
    println!("other.com/page1 = {:?}", art.get(b"http://other.com/page1"));
    println!("Count: {}\n", art.len());
}

fn example_frozen_layer() {
    println!("=== FrozenLayer (Immutable FST) ===\n");

    // Keys must be sorted for FST construction
    let mut data: Vec<(&[u8], u64)> = vec![
        (b"apple", 1),
        (b"banana", 2),
        (b"cherry", 3),
        (b"date", 4),
        (b"elderberry", 5),
    ];
    data.sort_by_key(|(k, _)| *k);

    let frozen = FrozenLayer::from_sorted_iter(data).unwrap();

    // Lookups
    println!("apple = {:?}", frozen.get(b"apple"));
    println!("cherry = {:?}", frozen.get(b"cherry"));
    println!("grape = {:?}", frozen.get(b"grape"));

    // Stats
    let stats = frozen.stats();
    println!("\nFST size: {} bytes", stats.fst_bytes);
    println!("Bytes per key: {:.1}", stats.bytes_per_key);
    println!("Key count: {}", stats.key_count);

    // Prefix scan
    println!("\nPrefix scan for 'c':");
    for (key, value) in frozen.prefix_scan(b"c") {
        println!("  {} = {}", String::from_utf8_lossy(&key), value);
    }

    // Range query
    println!("\nRange [b, d):");
    for (key, value) in frozen.range(b"b", b"d") {
        println!("  {} = {}", String::from_utf8_lossy(&key), value);
    }
}
