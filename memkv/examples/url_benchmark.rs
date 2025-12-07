//! Benchmark with realistic URLs - shuffled random insert order
//! Reports ACTUAL memory usage measured by jemalloc

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    // Load and shuffle URLs
    let urls_raw = std::fs::read_to_string("data/urls.txt").expect("Run from memkv directory");
    let mut urls: Vec<&str> = urls_raw.lines().collect();
    
    // Shuffle for random insert order
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    urls.sort_by_key(|s| {
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        h.finish()
    });
    
    let count = urls.len();
    let raw_key_bytes: usize = urls.iter().map(|u| u.len()).sum();
    
    println!("=== URL Benchmark ({} URLs, shuffled) ===", count);
    println!("Raw key bytes: {} ({:.1} MB, {:.1} avg/key)\n", 
             raw_key_bytes, raw_key_bytes as f64 / 1e6, raw_key_bytes as f64 / count as f64);
    
    println!("{:<15} {:>12} {:>12} {:>12} {:>10} {:>10}", 
             "Structure", "Total MB", "Overhead MB", "B/K (total)", "Insert/s", "Lookup/s");
    println!("{}", "-".repeat(80));
    
    // BTreeMap<Vec<u8>, u64> - baseline
    {
        let before = get_allocated();
        let start = Instant::now();
        let mut map: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes().to_vec(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url.as_bytes()) == Some(&(i as u64)) { found += 1; }
        }
        let lookup_time = start.elapsed();
        
        let total = after - before;
        let overhead = total as f64 - raw_key_bytes as f64;
        let bpk = total as f64 / count as f64;
        
        println!("{:<15} {:>12.2} {:>12.2} {:>12.1} {:>10.0} {:>10.0} {}", 
                 "BTreeMap",
                 total as f64 / 1e6,
                 overhead / 1e6,
                 bpk,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
    }
    
    // InlineHot - minimum overhead
    {
        let before = get_allocated();
        let start = Instant::now();
        let mut map = memkv::InlineHot::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        
        // Shrink to get actual usage
        map.shrink_to_fit();
        let after_shrink = get_allocated();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_time = start.elapsed();
        
        let total = after - before;
        let total_shrunk = after_shrink - before;
        let overhead = total_shrunk as f64 - raw_key_bytes as f64;
        let bpk = total_shrunk as f64 / count as f64;
        
        println!("{:<15} {:>12.2} {:>12.2} {:>12.1} {:>10.0} {:>10.0} {}", 
                 "InlineHot",
                 total_shrunk as f64 / 1e6,
                 overhead / 1e6,
                 bpk,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
        
        // Show actual internal usage
        let actual = map.memory_usage_actual();
        let index_only = actual - raw_key_bytes - count * 8; // subtract keys and values
        println!("  InlineHot actual: {:.2} MB, index-only: {:.2} MB ({:.1} B/K)",
                 actual as f64 / 1e6, index_only as f64 / 1e6, index_only as f64 / count as f64);
    }
    
    // HOT
    {
        let before = get_allocated();
        let start = Instant::now();
        let mut map = memkv::HOT::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        
        map.shrink_to_fit();
        let after_shrink = get_allocated();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_time = start.elapsed();
        
        let total_shrunk = after_shrink - before;
        let overhead = total_shrunk as f64 - raw_key_bytes as f64;
        let bpk = total_shrunk as f64 / count as f64;
        
        println!("{:<15} {:>12.2} {:>12.2} {:>12.1} {:>10.0} {:>10.0} {}", 
                 "HOT",
                 total_shrunk as f64 / 1e6,
                 overhead / 1e6,
                 bpk,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
        
        // Show actual internal usage
        let actual = map.memory_usage_actual();
        // HOT stores: key_data (keys only), leaves (key_off + key_len + value), nodes
        // Index overhead = leaves + nodes - values
        let index_only = actual - raw_key_bytes - count * 8;
        println!("  HOT actual: {:.2} MB, index-only: {:.2} MB ({:.1} B/K)",
                 actual as f64 / 1e6, index_only as f64 / 1e6, index_only as f64 / count as f64);
    }
    
    // FastArt - maximum speed
    {
        let before = get_allocated();
        let start = Instant::now();
        let mut map = memkv::FastArt::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_time = start.elapsed();
        
        let total = after - before;
        let overhead = total as f64 - raw_key_bytes as f64;
        let bpk = total as f64 / count as f64;
        
        println!("{:<15} {:>12.2} {:>12.2} {:>12.1} {:>10.0} {:>10.0} {}", 
                 "FastArt",
                 total as f64 / 1e6,
                 overhead / 1e6,
                 bpk,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
    }
    
    println!();
    println!("Notes:");
    println!("  - Total MB: actual memory used (jemalloc measured)");
    println!("  - Overhead MB: Total - raw key bytes");
    println!("  - B/K (total): bytes per key INCLUDING keys and values");
    println!();
    println!("HOT paper definition (11-14 B/K) counts:");
    println!("  - Index structure (nodes + child pointers)");
    println!("  - Does NOT count raw key storage");
}
