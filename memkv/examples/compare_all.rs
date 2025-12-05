//! Complete comparison of all implementations using jemalloc

use std::collections::BTreeMap;
use tikv_jemalloc_ctl::{epoch, stats};

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated_bytes() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn main() {
    let content = std::fs::read_to_string("urls_sample.txt").unwrap();
    let urls: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let count = urls.len();
    let data_size: usize = urls.iter().map(|s| s.len()).sum();
    
    println!("=== Complete Memory Comparison ===");
    println!("Dataset: {} URLs, {} MB raw key data", count, data_size / (1024 * 1024));
    println!("Using jemalloc for accurate allocation tracking\n");
    
    // Baseline
    let baseline = get_allocated_bytes();
    println!("Baseline (URLs loaded): {} MB\n", baseline / (1024 * 1024));
    
    // Results storage
    let mut results: Vec<(&str, usize, usize, f64)> = Vec::new();
    
    // ========== BTreeMap ==========
    println!("--- Testing BTreeMap ---");
    let before = get_allocated_bytes();
    let mut btree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for (i, url) in urls.iter().enumerate() {
        btree.insert(url.as_bytes().to_vec(), i as u64);
    }
    let after = get_allocated_bytes();
    let alloc = after - before;
    
    let correct = urls.iter().enumerate()
        .filter(|(i, url)| btree.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    println!("  Allocated: {} MB ({:.1} bytes/key)", alloc / (1024 * 1024), alloc as f64 / count as f64);
    println!("  Correctness: {}/{}", correct, count);
    results.push(("BTreeMap", alloc, correct, alloc as f64 / count as f64));
    drop(btree);
    let _ = get_allocated_bytes();
    
    // ========== ArenaArt ==========
    println!("\n--- Testing ArenaArt ---");
    use memkv::ArenaArt;
    
    let before = get_allocated_bytes();
    let mut tree: ArenaArt<u64> = ArenaArt::new();
    for (i, url) in urls.iter().enumerate() {
        tree.insert(url.as_bytes(), i as u64);
    }
    let after = get_allocated_bytes();
    let alloc = after - before;
    
    let correct = urls.iter().enumerate()
        .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    let stats = tree.memory_stats();
    println!("  Allocated: {} MB ({:.1} bytes/key)", alloc / (1024 * 1024), alloc as f64 / count as f64);
    println!("  Correctness: {}/{}", correct, count);
    println!("  Breakdown: data={} MB, nodes={} MB", 
             stats.data_arena_bytes / (1024 * 1024), 
             stats.node_arena_capacity / (1024 * 1024));
    results.push(("ArenaArt", alloc, correct, alloc as f64 / count as f64));
    drop(tree);
    let _ = get_allocated_bytes();
    
    // ========== UltraCompactArt ==========
    println!("\n--- Testing UltraCompactArt ---");
    use memkv::UltraCompactArt;
    
    let before = get_allocated_bytes();
    let mut tree: UltraCompactArt<u64> = UltraCompactArt::new();
    for (i, url) in urls.iter().enumerate() {
        tree.insert(url.as_bytes(), i as u64);
    }
    let after = get_allocated_bytes();
    let alloc = after - before;
    
    let correct = urls.iter().enumerate()
        .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    let stats = tree.memory_stats();
    println!("  Allocated: {} MB ({:.1} bytes/key)", alloc / (1024 * 1024), alloc as f64 / count as f64);
    println!("  Correctness: {}/{}", correct, count);
    println!("  Arena: {} MB", stats.arena_bytes / (1024 * 1024));
    results.push(("UltraCompactArt", alloc, correct, alloc as f64 / count as f64));
    drop(tree);
    let _ = get_allocated_bytes();
    
    // ========== FrozenLayer (FST) ==========
    println!("\n--- Testing FrozenLayer (FST) ---");
    use memkv::FrozenLayer;
    
    // FST requires sorted input
    let mut sorted_urls: Vec<(Vec<u8>, u64)> = urls.iter()
        .enumerate()
        .map(|(i, url)| (url.as_bytes().to_vec(), i as u64))
        .collect();
    sorted_urls.sort_by(|a, b| a.0.cmp(&b.0));
    
    let before = get_allocated_bytes();
    let frozen = FrozenLayer::from_sorted_iter(
        sorted_urls.iter().map(|(k, v)| (k.as_slice(), *v))
    ).unwrap();
    let after = get_allocated_bytes();
    let alloc = after - before;
    
    // Build lookup map for correctness check (since values are reordered)
    let lookup: std::collections::HashMap<Vec<u8>, u64> = sorted_urls.iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    
    let correct = urls.iter().enumerate()
        .filter(|(i, url)| {
            let expected = *i as u64;
            frozen.get(url.as_bytes()) == Some(expected)
        })
        .count();
    
    let stats = frozen.stats();
    println!("  Allocated: {} MB ({:.1} bytes/key)", alloc / (1024 * 1024), alloc as f64 / count as f64);
    println!("  FST size: {} MB ({:.1} bytes/key)", stats.fst_bytes / (1024 * 1024), stats.bytes_per_key);
    println!("  Correctness: {}/{}", correct, count);
    println!("  Compression vs raw: {:.1}x", data_size as f64 / stats.fst_bytes as f64);
    results.push(("FrozenLayer (FST)", alloc, correct, alloc as f64 / count as f64));
    
    // ========== Summary ==========
    println!("\n{}", "=".repeat(60));
    println!("=== SUMMARY ===");
    println!("{:<20} {:>12} {:>15} {:>12}", "Implementation", "Memory", "Bytes/Key", "Correctness");
    println!("{}", "-".repeat(60));
    
    results.sort_by(|a, b| a.1.cmp(&b.1));
    
    for (name, alloc, correct, bytes_per_key) in &results {
        let mem_mb = *alloc as f64 / (1024.0 * 1024.0);
        println!("{:<20} {:>10.1} MB {:>13.1} {:>10}/{}", 
                 name, mem_mb, bytes_per_key, correct, count);
    }
    
    println!("\nWinner: {} ({:.1} bytes/key)", results[0].0, results[0].3);
    
    if results.len() >= 2 {
        let savings = 100.0 * (1.0 - results[0].1 as f64 / results[1].1 as f64);
        println!("Saves {:.1}% vs next best ({})", savings, results[1].0);
    }
}
