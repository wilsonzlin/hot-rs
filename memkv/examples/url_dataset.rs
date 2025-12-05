//! Test with URL dataset.
//!
//! This example loads URLs from a file and benchmarks memory usage.
//! 
//! Usage:
//!   cargo run --release --example url_dataset -- <path_to_urls>
//!
//! To get sample data:
//!   curl -r 0-10000000 "https://static.wilsonl.in/urls.txt" > urls_10mb.txt

use memkv::MemKV;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: url_dataset <path_to_urls>");
        eprintln!("  Download sample: curl -r 0-10000000 'https://static.wilsonl.in/urls.txt' > urls.txt");
        std::process::exit(1);
    });

    println!("Loading URLs from: {}", path);
    
    let file = File::open(&path).expect("Failed to open file");
    let reader = BufReader::new(file);
    let urls: Vec<String> = reader.lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.is_empty())
        .collect();

    let n = urls.len();
    println!("Loaded {} URLs", n);
    
    if n == 0 {
        println!("No URLs to process");
        return;
    }

    // Compute statistics about the URLs
    let total_bytes: usize = urls.iter().map(|u| u.len()).sum();
    let avg_len = total_bytes as f64 / n as f64;
    let min_len = urls.iter().map(|u| u.len()).min().unwrap();
    let max_len = urls.iter().map(|u| u.len()).max().unwrap();
    
    println!("\nURL Statistics:");
    println!("  Total raw bytes: {} ({:.2} MB)", total_bytes, total_bytes as f64 / 1_000_000.0);
    println!("  Average length: {:.1} bytes", avg_len);
    println!("  Min length: {} bytes", min_len);
    println!("  Max length: {} bytes", max_len);

    // Test BTreeMap
    println!("\n=== BTreeMap<String, u64> ===");
    let before = get_memory_usage();
    let start = Instant::now();
    let mut btree: BTreeMap<String, u64> = BTreeMap::new();
    for (i, url) in urls.iter().enumerate() {
        btree.insert(url.clone(), i as u64);
    }
    let insert_time = start.elapsed();
    let after = get_memory_usage();
    let btree_mem = after.saturating_sub(before);
    
    println!("  Insert time: {:?}", insert_time);
    println!("  Memory used: {} bytes ({:.2} MB)", btree_mem, btree_mem as f64 / 1_000_000.0);
    println!("  Bytes per key: {:.2}", btree_mem as f64 / n as f64);
    println!("  Overhead vs raw: {:.2}x", btree_mem as f64 / total_bytes as f64);

    // Benchmark lookups
    let start = Instant::now();
    let mut found = 0;
    for url in urls.iter().take(10000) {
        if btree.get(url).is_some() {
            found += 1;
        }
    }
    let lookup_time = start.elapsed();
    println!("  Lookup: {} in {:?} ({:.0} ops/sec)", found, lookup_time, 
        10000.0 / lookup_time.as_secs_f64());

    drop(btree);
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Test MemKV
    println!("\n=== MemKV<u64> ===");
    let before = get_memory_usage();
    let start = Instant::now();
    let kv: MemKV<u64> = MemKV::new();
    for (i, url) in urls.iter().enumerate() {
        kv.insert(url.as_bytes(), i as u64);
    }
    let insert_time = start.elapsed();
    let after = get_memory_usage();
    let memkv_mem = after.saturating_sub(before);
    let stats = kv.memory_usage();
    
    println!("  Insert time: {:?}", insert_time);
    println!("  Memory used (RSS): {} bytes ({:.2} MB)", memkv_mem, memkv_mem as f64 / 1_000_000.0);
    println!("  Bytes per key (RSS): {:.2}", memkv_mem as f64 / n as f64);
    println!("  Internal stats:");
    println!("    Key bytes: {}", stats.key_bytes);
    println!("    Node bytes: {}", stats.node_bytes);
    println!("    Bytes per key (internal): {:.2}", stats.bytes_per_key);
    println!("  Overhead vs raw: {:.2}x", memkv_mem as f64 / total_bytes as f64);

    // Benchmark lookups
    let start = Instant::now();
    let mut found = 0;
    for url in urls.iter().take(10000) {
        if kv.get(url.as_bytes()).is_some() {
            found += 1;
        }
    }
    let lookup_time = start.elapsed();
    println!("  Lookup: {} in {:?} ({:.0} ops/sec)", found, lookup_time,
        10000.0 / lookup_time.as_secs_f64());

    // Verify a sample
    println!("\nVerifying correctness...");
    let mut correct = 0;
    for (i, url) in urls.iter().enumerate().take(1000) {
        if kv.get(url.as_bytes()) == Some(i as u64) {
            correct += 1;
        }
    }
    println!("  Correct: {}/1000", correct);

    // Test prefix scan
    if n > 0 {
        println!("\nPrefix scan test...");
        // Find a common prefix
        let first_url = &urls[0];
        if let Some(slash_pos) = first_url.find('/') {
            let prefix = &first_url[..slash_pos + 1];
            let start = Instant::now();
            let count = kv.prefix(prefix.as_bytes()).len();
            let scan_time = start.elapsed();
            println!("  Prefix '{}' returned {} results in {:?}", prefix, count, scan_time);
        }
    }

    // Summary
    if btree_mem > 0 && memkv_mem > 0 {
        println!("\n=== SUMMARY ===");
        println!("Keys: {}", n);
        println!("Raw data: {:.2} MB", total_bytes as f64 / 1_000_000.0);
        println!("BTreeMap: {:.2} MB ({:.2} bytes/key)", 
            btree_mem as f64 / 1_000_000.0, btree_mem as f64 / n as f64);
        println!("MemKV: {:.2} MB ({:.2} bytes/key)", 
            memkv_mem as f64 / 1_000_000.0, memkv_mem as f64 / n as f64);
        println!("Savings: {:.1}x", btree_mem as f64 / memkv_mem as f64);
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
