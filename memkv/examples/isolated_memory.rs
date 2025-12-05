//! Measure UltraCompactArt memory in isolation.

use memkv::UltraCompactArt;
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
    println!("=== Isolated UltraCompactArt Memory Test ===\n");
    
    // Measure baseline BEFORE any major allocations
    let baseline = get_process_memory();
    println!("Baseline memory: {} MB", baseline / (1024 * 1024));
    
    // Create tree and insert directly from file (no intermediate storage)
    let mut tree: UltraCompactArt<u64> = UltraCompactArt::new();
    let mut count = 0;
    
    if let Ok(file) = File::open("urls_sample.txt") {
        for (i, line) in BufReader::new(file).lines().enumerate() {
            if let Ok(url) = line {
                tree.insert(url.as_bytes(), i as u64);
                count += 1;
            }
        }
    } else {
        // Use synthetic data
        for i in 0..100_000u64 {
            let key = format!("https://example{}.com/path/{}/page/{}", i % 100, i / 100, i);
            tree.insert(key.as_bytes(), i);
            count += 1;
        }
    }
    
    let after = get_process_memory();
    let used = after - baseline;
    
    println!("Keys: {}", count);
    println!("After insertion: {} MB", after / (1024 * 1024));
    println!("Memory used: {} MB ({:.2} bytes/key)", 
             used / (1024 * 1024), 
             used as f64 / count as f64);
    
    // Internal stats
    let stats = tree.memory_stats();
    println!("\nInternal stats:");
    println!("  Arena: {} KB", stats.arena_bytes / 1024);
    println!("  Nodes (estimated): {} KB", stats.node_bytes / 1024);
    
    // Node counts
    println!("\nNodes:");
    println!("  Leaves: {}", stats.leaf_count);
    println!("  Node4: {}", stats.node4_count);
    println!("  Node16: {}", stats.node16_count);
    println!("  Node48: {}", stats.node48_count);
    println!("  Node256: {}", stats.node256_count);
    
    let total = stats.leaf_count + stats.node4_count + stats.node16_count + 
                stats.node48_count + stats.node256_count;
    println!("  Total: {}", total);
    let node_size = std::mem::size_of::<memkv::UltraNode<u64>>();
    println!("  Node size (UltraNode<u64>): {} bytes", node_size);
    println!("  Estimated node memory: {} MB", total * node_size / (1024 * 1024));
    
    // Verify correctness (sample)
    if let Ok(file) = File::open("urls_sample.txt") {
        let mut correct = 0;
        for (i, line) in BufReader::new(file).lines().enumerate() {
            if let Ok(url) = line {
                if tree.get(url.as_bytes()) == Some(&(i as u64)) {
                    correct += 1;
                }
            }
        }
        println!("\nCorrectness: {}/{}", correct, count);
    }
}
