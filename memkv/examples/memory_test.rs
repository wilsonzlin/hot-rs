//! Memory efficiency test.

use memkv::MemKV;
use std::collections::BTreeMap;

fn main() {
    let n = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);

    println!("Testing with {} keys", n);
    println!("=====================\n");

    // Test BTreeMap baseline
    println!("BTreeMap<String, u64>:");
    let before = get_memory_usage();
    let mut btree: BTreeMap<String, u64> = BTreeMap::new();
    for i in 0..n {
        let key = format!("user:{:08}", i);
        btree.insert(key, i as u64);
    }
    let after = get_memory_usage();
    let btree_mem = after.saturating_sub(before);
    println!("  Memory used: {} bytes", btree_mem);
    println!("  Bytes per key: {:.2}", btree_mem as f64 / n as f64);
    drop(btree);

    // Force garbage collection / memory return
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Test MemKV
    println!("\nMemKV<u64>:");
    let before = get_memory_usage();
    let kv: MemKV<u64> = MemKV::new();
    for i in 0..n {
        let key = format!("user:{:08}", i);
        kv.insert(key.as_bytes(), i as u64);
    }
    let after = get_memory_usage();
    let memkv_mem = after.saturating_sub(before);
    let stats = kv.memory_usage();
    
    println!("  Memory used (RSS delta): {} bytes", memkv_mem);
    println!("  Bytes per key (RSS): {:.2}", memkv_mem as f64 / n as f64);
    println!("  Internal stats:");
    println!("    Key bytes: {}", stats.key_bytes);
    println!("    Node bytes: {}", stats.node_bytes);
    println!("    Value bytes: {}", stats.value_bytes);
    println!("    Bytes per key (internal): {:.2}", stats.bytes_per_key);

    // Verify correctness
    println!("\nVerifying correctness...");
    let mut correct = 0;
    for i in 0..n {
        let key = format!("user:{:08}", i);
        if kv.get(key.as_bytes()) == Some(i as u64) {
            correct += 1;
        }
    }
    println!("  Correct lookups: {}/{}", correct, n);

    // Comparison
    if btree_mem > 0 && memkv_mem > 0 {
        println!("\n=== Summary ===");
        println!("Memory savings: {:.1}x ({} bytes vs {} bytes)",
            btree_mem as f64 / memkv_mem as f64,
            btree_mem,
            memkv_mem,
        );
    }
}

fn get_memory_usage() -> usize {
    // Try to read from /proc/self/statm on Linux
    if let Ok(content) = std::fs::read_to_string("/proc/self/statm") {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if let Some(rss) = parts.get(1) {
            if let Ok(pages) = rss.parse::<usize>() {
                return pages * 4096; // Assume 4KB pages
            }
        }
    }
    0
}
