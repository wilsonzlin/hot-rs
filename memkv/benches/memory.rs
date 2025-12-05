//! Memory efficiency benchmarks.
//!
//! This benchmark measures memory usage per key for different data patterns.

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use memkv::MemKV;
use std::collections::BTreeMap;

fn measure_memkv_memory(keys: &[String]) -> (usize, f64) {
    let kv: MemKV<u64> = MemKV::new();
    for (i, key) in keys.iter().enumerate() {
        kv.insert(key.as_bytes(), i as u64);
    }
    let stats = kv.memory_usage();
    let total = stats.key_bytes + stats.node_bytes + stats.value_bytes;
    (total, stats.bytes_per_key)
}

fn measure_btreemap_memory(keys: &[String]) -> usize {
    // Approximate BTreeMap memory usage
    // Each entry: key (24 + len) + value (8) + node overhead (~16)
    keys.iter().map(|k| 24 + k.len() + 8 + 16).sum()
}

fn bench_memory_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_patterns");
    group.sample_size(10);
    
    let sizes = [1_000, 10_000, 100_000];
    
    for size in sizes {
        // Sequential keys (high prefix sharing)
        let sequential: Vec<String> = (0..size)
            .map(|i| format!("user:profile:settings:{:08}", i))
            .collect();
        
        group.bench_with_input(
            BenchmarkId::new("sequential", size),
            &sequential,
            |b, keys| {
                b.iter(|| measure_memkv_memory(keys))
            },
        );
        
        // UUID-like keys (low prefix sharing)
        let uuid_like: Vec<String> = (0..size)
            .map(|i| format!("{:08x}-{:04x}-{:04x}-{:04x}-{:012x}", 
                i, i % 0xFFFF, (i * 7) % 0xFFFF, (i * 13) % 0xFFFF, i * 31))
            .collect();
        
        group.bench_with_input(
            BenchmarkId::new("uuid_like", size),
            &uuid_like,
            |b, keys| {
                b.iter(|| measure_memkv_memory(keys))
            },
        );
        
        // URL-like keys (moderate prefix sharing)
        let domains = ["example.com", "test.org", "demo.net"];
        let url_like: Vec<String> = (0..size)
            .map(|i| format!("{}/users/{}/posts/{}", 
                domains[i % domains.len()], i / 100, i % 100))
            .collect();
        
        group.bench_with_input(
            BenchmarkId::new("url_like", size),
            &url_like,
            |b, keys| {
                b.iter(|| measure_memkv_memory(keys))
            },
        );
    }
    
    group.finish();
}

fn print_memory_report() {
    println!("\n=== Memory Efficiency Report ===\n");
    
    let sizes = [1_000, 10_000, 100_000];
    
    for size in sizes {
        println!("--- {} keys ---", size);
        
        // Sequential
        let keys: Vec<String> = (0..size)
            .map(|i| format!("user:profile:settings:{:08}", i))
            .collect();
        let raw_bytes: usize = keys.iter().map(|k| k.len()).sum();
        let (memkv_bytes, bytes_per_key) = measure_memkv_memory(&keys);
        let btree_bytes = measure_btreemap_memory(&keys);
        
        println!("Sequential (high prefix sharing):");
        println!("  Raw: {} bytes ({:.1} bytes/key)", raw_bytes, raw_bytes as f64 / size as f64);
        println!("  MemKV: {} bytes ({:.1} bytes/key)", memkv_bytes, bytes_per_key);
        println!("  BTreeMap (est): {} bytes ({:.1} bytes/key)", btree_bytes, btree_bytes as f64 / size as f64);
        println!("  Savings vs BTreeMap: {:.1}x", btree_bytes as f64 / memkv_bytes as f64);
        
        // URL-like
        let domains = ["example.com", "test.org", "demo.net"];
        let keys: Vec<String> = (0..size)
            .map(|i| format!("{}/users/{}/posts/{}", 
                domains[i % domains.len()], i / 100, i % 100))
            .collect();
        let raw_bytes: usize = keys.iter().map(|k| k.len()).sum();
        let (memkv_bytes, bytes_per_key) = measure_memkv_memory(&keys);
        let btree_bytes = measure_btreemap_memory(&keys);
        
        println!("\nURL-like (moderate prefix sharing):");
        println!("  Raw: {} bytes ({:.1} bytes/key)", raw_bytes, raw_bytes as f64 / size as f64);
        println!("  MemKV: {} bytes ({:.1} bytes/key)", memkv_bytes, bytes_per_key);
        println!("  BTreeMap (est): {} bytes ({:.1} bytes/key)", btree_bytes, btree_bytes as f64 / size as f64);
        println!("  Savings vs BTreeMap: {:.1}x", btree_bytes as f64 / memkv_bytes as f64);
        
        println!();
    }
}

fn bench_memory(c: &mut Criterion) {
    // Print report once
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(print_memory_report);
    
    bench_memory_patterns(c);
}

criterion_group!(benches, bench_memory);
criterion_main!(benches);
