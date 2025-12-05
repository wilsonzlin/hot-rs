//! Final comparison of memory usage

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

fn get_process_memory() -> usize {
    if let Ok(contents) = std::fs::read_to_string("/proc/self/statm") {
        let parts: Vec<&str> = contents.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(pages) = parts[1].parse::<usize>() {
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize };
                return pages * page_size;
            }
        }
    }
    0
}

fn main() {
    let content = std::fs::read_to_string("urls_sample.txt").unwrap();
    let urls: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let count = urls.len();
    
    println!("=== Memory Comparison ({} URLs) ===\n", count);
    
    // Test BTreeMap
    let baseline = get_process_memory();
    let mut btree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for (i, url) in urls.iter().enumerate() {
        btree.insert(url.as_bytes().to_vec(), i as u64);
    }
    let btree_mem = get_process_memory() - baseline;
    let btree_bytes_per_key = btree_mem as f64 / count as f64;
    
    // Verify correctness
    let btree_correct = urls.iter().enumerate()
        .filter(|(i, url)| btree.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    drop(btree);
    
    println!("BTreeMap:");
    println!("  Memory: {} MB", btree_mem / (1024 * 1024));
    println!("  Bytes/key: {:.1}", btree_bytes_per_key);
    println!("  Correctness: {}/{}\n", btree_correct, count);
    
    // Test UltraCompactArt
    use memkv::UltraCompactArt;
    
    let baseline = get_process_memory();
    let mut ultra: UltraCompactArt<u64> = UltraCompactArt::new();
    for (i, url) in urls.iter().enumerate() {
        ultra.insert(url.as_bytes(), i as u64);
    }
    let ultra_mem = get_process_memory() - baseline;
    let ultra_bytes_per_key = ultra_mem as f64 / count as f64;
    
    // Verify correctness
    let ultra_correct = urls.iter().enumerate()
        .filter(|(i, url)| ultra.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    // Get internal stats
    let stats = ultra.memory_stats();
    
    println!("UltraCompactArt:");
    println!("  Memory (RSS): {} MB", ultra_mem / (1024 * 1024));
    println!("  Bytes/key: {:.1}", ultra_bytes_per_key);
    println!("  Correctness: {}/{}", ultra_correct, count);
    println!("  Internal arena: {} KB", stats.arena_bytes / 1024);
    println!("  Internal nodes: {} KB", stats.node_bytes / 1024);
    println!("  Nodes: {} leaves, {} Node4, {} Node16, {} Node48, {} Node256",
             stats.leaf_count, stats.node4_count, stats.node16_count, 
             stats.node48_count, stats.node256_count);
    
    println!("\n=== Comparison ===");
    let savings = (1.0 - ultra_mem as f64 / btree_mem as f64) * 100.0;
    println!("  vs BTreeMap: {:.0}% {}", 
             savings.abs(),
             if savings > 0.0 { "less memory" } else { "more memory" });
}
