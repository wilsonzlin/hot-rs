//! Test minimal overhead structures
use std::io::{BufRead, BufReader};
use std::fs::File;
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

fn main() {
    let limit: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    
    println!("Loading {} URLs...", limit);
    let file = File::open("/tmp/urls_sample.txt").unwrap();
    let reader = BufReader::new(file);
    let mut keys: Vec<String> = reader.lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.is_empty() && l != "url")
        .take(limit)
        .collect();
    
    let key_count = keys.len();
    let total_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    let avg_key_len = total_key_bytes as f64 / key_count as f64;
    
    // Sort keys
    keys.sort();
    
    println!("Loaded {} keys, {:.1} MB total, avg {:.1} bytes/key\n", 
             key_count, total_key_bytes as f64 / 1e6, avg_key_len);
    
    println!("╔════════════════════════════════════════════════════════════════════╗");
    println!("║           MINIMAL OVERHEAD BENCHMARK ({} keys)            ║", key_count);
    println!("╠════════════════════════════════════════════════════════════════════╣");
    println!("║ Structure       │ Memory   │ Overhead   │ Insert/s │ Lookup/s │OK?║");
    println!("╠════════════════════════════════════════════════════════════════════╣");
    
    // MinimalSorted (2 bytes overhead - just key length!)
    {
        use memkv::MinimalSorted;
        let before = get_allocated();
        let mut store = MinimalSorted::with_capacity(total_key_bytes, key_count);
        let start = Instant::now();
        for key in &keys {
            store.push(key.as_bytes());
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if store.get(key.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ MinimalSorted   │ {:>6.1}MB │ {:>+6.1} B/K │ {:>8.0} │ {:>8.0} │ {} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // Compact32 (10 bytes overhead: 2 len + 4 value + 4 offset)
    {
        use memkv::Compact32;
        let before = get_allocated();
        let mut store = Compact32::with_capacity(key_count, total_key_bytes);
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            store.insert(key.as_bytes(), i as u32);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if store.get(key.as_bytes()) == Some(i as u32) { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ Compact32       │ {:>6.1}MB │ {:>+6.1} B/K │ {:>8.0} │ {:>8.0} │ {} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // GLORY for comparison
    {
        use memkv::Glory;
        let before = get_allocated();
        let mut store = memkv::Glory::with_capacity(key_count, total_key_bytes + key_count * 2);
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            store.insert(key.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if store.get(key.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ GLORY           │ {:>6.1}MB │ {:>+6.1} B/K │ {:>8.0} │ {:>8.0} │ {} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // FST for comparison
    {
        use memkv::FrozenLayer;
        let before = get_allocated();
        let start = Instant::now();
        let store = FrozenLayer::from_sorted_iter(
            keys.iter().enumerate().map(|(i, k)| (k.as_bytes(), i as u64))
        ).unwrap();
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in &keys {
            if store.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ FST             │ {:>6.1}MB │ {:>+6.1} B/K │ {:>8.0} │ {:>8.0} │ {} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    println!("╚════════════════════════════════════════════════════════════════════╝");
    println!("\nTheoretical minimum overhead:");
    println!("  MinimalSorted: 2 bytes (just key length)");
    println!("  Compact32: 10 bytes (2 len + 4 value + 4 offset)");
    println!("  GLORY: 14 bytes (2 len + 8 value + 4 offset)");
    println!("  HOT target: 11-14 bytes");
}
