//! Final comparison of memory usage
//! 
//! Note: This measures each structure from a fresh baseline.
//! Due to allocator behavior, the second measurement may reuse
//! memory from the first. For accurate isolated measurements,
//! run each test in a separate process.

use std::collections::BTreeMap;

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
    let data_size = urls.iter().map(|s| s.len()).sum::<usize>();
    
    println!("=== Memory Comparison ({} URLs, {} MB raw data) ===\n", 
             count, data_size / (1024 * 1024));
    
    // Initial baseline (includes url Vec)
    let initial_baseline = get_process_memory();
    println!("Baseline (program + URL data): {} MB\n", initial_baseline / (1024 * 1024));
    
    // Test BTreeMap first
    let before_btree = get_process_memory();
    let mut btree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for (i, url) in urls.iter().enumerate() {
        btree.insert(url.as_bytes().to_vec(), i as u64);
    }
    let after_btree = get_process_memory();
    let btree_mem = after_btree - before_btree;
    
    // Verify correctness
    let btree_correct = urls.iter().enumerate()
        .filter(|(i, url)| btree.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    println!("BTreeMap:");
    println!("  Memory delta: {} MB ({:.1} bytes/key)", 
             btree_mem / (1024 * 1024), btree_mem as f64 / count as f64);
    println!("  Correctness: {}/{}", btree_correct, count);
    
    // Keep btree alive, test UltraCompactArt
    use memkv::UltraCompactArt;
    
    let before_ultra = get_process_memory();
    let mut ultra: UltraCompactArt<u64> = UltraCompactArt::new();
    for (i, url) in urls.iter().enumerate() {
        ultra.insert(url.as_bytes(), i as u64);
    }
    let after_ultra = get_process_memory();
    let ultra_mem = after_ultra - before_ultra;
    
    // Verify correctness
    let ultra_correct = urls.iter().enumerate()
        .filter(|(i, url)| ultra.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    let stats = ultra.memory_stats();
    
    println!("\nUltraCompactArt:");
    println!("  Memory delta: {} MB ({:.1} bytes/key)", 
             ultra_mem / (1024 * 1024), ultra_mem as f64 / count as f64);
    println!("  Correctness: {}/{}", ultra_correct, count);
    println!("  Arena (keys+prefixes): {} MB", stats.arena_bytes / (1024 * 1024));
    
    // Total memory with both structures alive
    let total = get_process_memory();
    println!("\nTotal memory (both structures): {} MB", total / (1024 * 1024));
    println!("  BTreeMap contribution: {} MB", btree_mem / (1024 * 1024));
    println!("  UltraCompactArt contribution: {} MB", ultra_mem / (1024 * 1024));
    
    // Comparison
    println!("\n=== Comparison ===");
    if ultra_mem < btree_mem {
        let savings = 100.0 * (1.0 - ultra_mem as f64 / btree_mem as f64);
        println!("  UltraCompactArt uses {:.0}% less memory than BTreeMap", savings);
    } else {
        let increase = 100.0 * (ultra_mem as f64 / btree_mem as f64 - 1.0);
        println!("  UltraCompactArt uses {:.0}% more memory than BTreeMap", increase);
    }
    
    // Keep both alive
    println!("\nBTreeMap len: {}", btree.len());
    println!("UltraCompactArt len: {}", ultra.len());
}
