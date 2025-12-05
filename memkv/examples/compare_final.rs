//! Final comparison of memory usage using jemalloc for accurate stats

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
    
    println!("=== Memory Comparison ({} URLs, {} MB raw data) ===", 
             count, data_size / (1024 * 1024));
    println!("Using jemalloc for accurate allocation tracking\n");
    
    // Baseline after loading URLs
    let baseline = get_allocated_bytes();
    println!("Baseline (after loading URLs): {} MB\n", baseline / (1024 * 1024));
    
    // Test BTreeMap
    println!("--- BTreeMap ---");
    let before_btree = get_allocated_bytes();
    let mut btree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for (i, url) in urls.iter().enumerate() {
        btree.insert(url.as_bytes().to_vec(), i as u64);
    }
    let after_btree = get_allocated_bytes();
    let btree_alloc = after_btree - before_btree;
    
    let btree_correct = urls.iter().enumerate()
        .filter(|(i, url)| btree.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    println!("  Allocated: {} MB ({:.1} bytes/key)", 
             btree_alloc / (1024 * 1024), btree_alloc as f64 / count as f64);
    println!("  Correctness: {}/{}", btree_correct, count);
    
    drop(btree);
    let _ = get_allocated_bytes(); // Let jemalloc return memory
    
    // Test ArenaArt (new arena-based implementation)
    println!("\n--- ArenaArt (arena-based nodes) ---");
    use memkv::ArenaArt;
    
    let before_arena = get_allocated_bytes();
    let mut arena_art: ArenaArt<u64> = ArenaArt::new();
    for (i, url) in urls.iter().enumerate() {
        arena_art.insert(url.as_bytes(), i as u64);
    }
    let after_arena = get_allocated_bytes();
    let arena_alloc = after_arena - before_arena;
    
    let arena_correct = urls.iter().enumerate()
        .filter(|(i, url)| arena_art.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    let arena_stats = arena_art.memory_stats();
    
    println!("  Allocated: {} MB ({:.1} bytes/key)", 
             arena_alloc / (1024 * 1024), arena_alloc as f64 / count as f64);
    println!("  Correctness: {}/{}", arena_correct, count);
    println!("  Internal breakdown:");
    println!("    Data arena: {} MB", arena_stats.data_arena_bytes / (1024 * 1024));
    println!("    Node arena: {} MB", arena_stats.node_arena_capacity / (1024 * 1024));
    println!("    Nodes: {} total ({} leaves, {} N4, {} N16, {} N48, {} N256)",
             arena_stats.node_count,
             arena_stats.leaf_count, arena_stats.node4_count, 
             arena_stats.node16_count, arena_stats.node48_count, arena_stats.node256_count);
    
    drop(arena_art);
    let _ = get_allocated_bytes();
    
    // Test UltraCompactArt (Box-based nodes)
    println!("\n--- UltraCompactArt (Box-based nodes) ---");
    use memkv::UltraCompactArt;
    
    let before_ultra = get_allocated_bytes();
    let mut ultra: UltraCompactArt<u64> = UltraCompactArt::new();
    for (i, url) in urls.iter().enumerate() {
        ultra.insert(url.as_bytes(), i as u64);
    }
    let after_ultra = get_allocated_bytes();
    let ultra_alloc = after_ultra - before_ultra;
    
    let ultra_correct = urls.iter().enumerate()
        .filter(|(i, url)| ultra.get(url.as_bytes()) == Some(&(*i as u64)))
        .count();
    
    let ultra_stats = ultra.memory_stats();
    
    println!("  Allocated: {} MB ({:.1} bytes/key)", 
             ultra_alloc / (1024 * 1024), ultra_alloc as f64 / count as f64);
    println!("  Correctness: {}/{}", ultra_correct, count);
    println!("  Internal breakdown:");
    println!("    Arena: {} MB", ultra_stats.arena_bytes / (1024 * 1024));
    println!("    Nodes: {} leaves, {} N4, {} N16, {} N48, {} N256",
             ultra_stats.leaf_count, ultra_stats.node4_count, 
             ultra_stats.node16_count, ultra_stats.node48_count, ultra_stats.node256_count);
    
    // Summary
    println!("\n=== Summary ===");
    println!("  BTreeMap:        {} MB ({:.1} bytes/key)", 
             btree_alloc / (1024 * 1024), btree_alloc as f64 / count as f64);
    println!("  ArenaArt:        {} MB ({:.1} bytes/key)", 
             arena_alloc / (1024 * 1024), arena_alloc as f64 / count as f64);
    println!("  UltraCompactArt: {} MB ({:.1} bytes/key)", 
             ultra_alloc / (1024 * 1024), ultra_alloc as f64 / count as f64);
    
    if arena_alloc < btree_alloc {
        let savings = 100.0 * (1.0 - arena_alloc as f64 / btree_alloc as f64);
        println!("\n  ArenaArt uses {:.1}% LESS memory than BTreeMap!", savings);
    } else {
        let overhead = 100.0 * (arena_alloc as f64 / btree_alloc as f64 - 1.0);
        println!("\n  ArenaArt uses {:.1}% more memory than BTreeMap", overhead);
    }
    
    // Keep alive for measurement
    println!("\n  UltraCompact len: {}", ultra.len());
}
