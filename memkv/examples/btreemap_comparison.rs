//! Direct BTreeMap comparison - the main use case

use std::collections::BTreeMap;
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    
    // Generate realistic database keys (random order)
    println!("Generating {} database-style keys (random insert order)...", count);
    let keys: Vec<String> = (0..count)
        .map(|i| {
            match i % 7 {
                0 => format!("user:{:08x}", hash(i)),
                1 => format!("session:{}:data", hash(i)),
                2 => format!("cache:item:{}:{}", i / 100, hash(i)),
                3 => format!("db:users:row:{:012}", hash(i)),
                4 => format!("api:/v1/users/{}/profile", hash(i)),
                5 => format!("metric:cpu:host{}:{}", i % 100, hash(i)),
                _ => format!("object:{}", hash(i)),
            }
        })
        .collect();
    
    let total_bytes: usize = keys.iter().map(|k| k.len()).sum();
    let avg = total_bytes as f64 / count as f64;
    
    println!("Total key bytes: {:.1} MB, avg {:.1} bytes/key\n", total_bytes as f64 / 1e6, avg);
    
    println!("═══════════════════════════════════════════════════════════════════════════════");
    println!("                      BTreeMap vs FastArt: Random Inserts");
    println!("═══════════════════════════════════════════════════════════════════════════════\n");

    // BTreeMap<Vec<u8>, u64>
    let before = get_allocated();
    let mut btree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    let start = Instant::now();
    for (i, key) in keys.iter().enumerate() {
        btree.insert(key.as_bytes().to_vec(), i as u64);
    }
    let btree_insert_time = start.elapsed();
    let btree_mem = get_allocated().saturating_sub(before);
    
    let start = Instant::now();
    let mut btree_found = 0;
    for (i, key) in keys.iter().enumerate() {
        if btree.get(key.as_bytes()) == Some(&(i as u64)) { btree_found += 1; }
    }
    let btree_lookup_time = start.elapsed();

    // FastArt
    drop(btree);
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    
    let before = get_allocated();
    let mut art = memkv::FastArt::new();
    let start = Instant::now();
    for (i, key) in keys.iter().enumerate() {
        art.insert(key.as_bytes(), i as u64);
    }
    let art_insert_time = start.elapsed();
    let art_mem = get_allocated().saturating_sub(before);
    
    let start = Instant::now();
    let mut art_found = 0;
    for (i, key) in keys.iter().enumerate() {
        if art.get(key.as_bytes()) == Some(i as u64) { art_found += 1; }
    }
    let art_lookup_time = start.elapsed();
    
    let btree_overhead = (btree_mem as f64 - total_bytes as f64) / count as f64;
    let art_overhead = (art_mem as f64 - total_bytes as f64) / count as f64;
    let mem_savings = 100.0 * (1.0 - art_mem as f64 / btree_mem as f64);
    let insert_speedup = btree_insert_time.as_secs_f64() / art_insert_time.as_secs_f64();
    let lookup_speedup = btree_lookup_time.as_secs_f64() / art_lookup_time.as_secs_f64();
    
    println!("BTreeMap<Vec<u8>, u64>:");
    println!("  Memory:    {:.1} MB ({:+.1} bytes/key overhead)", btree_mem as f64 / 1e6, btree_overhead);
    println!("  Insert:    {:.1} ops/sec", count as f64 / btree_insert_time.as_secs_f64());
    println!("  Lookup:    {:.1} ops/sec", count as f64 / btree_lookup_time.as_secs_f64());
    println!("  Correct:   {} ({}%)", btree_found, 100 * btree_found / count);
    println!();
    
    println!("FastArt:");
    println!("  Memory:    {:.1} MB ({:+.1} bytes/key overhead)", art_mem as f64 / 1e6, art_overhead);
    println!("  Insert:    {:.1} ops/sec", count as f64 / art_insert_time.as_secs_f64());
    println!("  Lookup:    {:.1} ops/sec", count as f64 / art_lookup_time.as_secs_f64());
    println!("  Correct:   {} ({}%)", art_found, 100 * art_found / count);
    println!();
    
    println!("═══════════════════════════════════════════════════════════════════════════════");
    println!("                              IMPROVEMENT SUMMARY");
    println!("═══════════════════════════════════════════════════════════════════════════════");
    println!("  Memory:    {:.0}% LESS ({:.1} MB saved)", mem_savings, (btree_mem - art_mem) as f64 / 1e6);
    println!("  Insert:    {:.1}x FASTER", insert_speedup);
    println!("  Lookup:    {:.1}x FASTER", lookup_speedup);
    println!("═══════════════════════════════════════════════════════════════════════════════");
}

fn hash(x: usize) -> u64 {
    let mut v = x as u64;
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51afd7ed558ccd);
    v ^= v >> 33;
    v = v.wrapping_mul(0xc4ceb9fe1a85ec53);
    v ^= v >> 33;
    v
}
