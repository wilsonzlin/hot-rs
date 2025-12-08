//! Scale test - 500K URLs

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let urls_raw = std::fs::read_to_string("data/urls_500k.txt").expect("Need urls_500k.txt");
    let mut urls: Vec<&str> = urls_raw.lines().collect();
    
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    urls.sort_by_key(|s| {
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        h.finish()
    });
    
    let count = urls.len();
    let raw = urls.iter().map(|u| u.len()).sum::<usize>();
    
    println!("╔═══════════════════════════════════════════════════════════════════════╗");
    println!("║          SCALE TEST - {} URLs (shuffled random insert)          ║", count);
    println!("║                   Raw key data: {:.1} MB                              ║", raw as f64/1e6);
    println!("╚═══════════════════════════════════════════════════════════════════════╝\n");
    
    println!("{:<12} {:>10} {:>10} {:>10} {:>12} {:>10}", 
             "Structure", "Total MB", "Overhead", "Index", "Insert/s", "Lookup/s");
    println!("{:<12} {:>10} {:>10} {:>10} {:>12} {:>10}",
             "", "", "B/K", "B/K", "", "");
    println!("{}", "─".repeat(70));
    
    // BTreeMap
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
        
        let total = (after - before) as f64;
        let overhead = (total - raw as f64) / count as f64;
        let index = overhead - 8.0;
        
        println!("{:<12} {:>10.1} {:>10.1} {:>10.1} {:>12.0} {:>10.0} {}", 
                 "BTreeMap", total/1e6, overhead, index,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
    }
    
    // InlineHot
    {
        let before = get_allocated();
        let start = Instant::now();
        let mut map = memkv::InlineHot::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        map.shrink_to_fit();
        let after = get_allocated();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_time = start.elapsed();
        
        let total = (after - before) as f64;
        let overhead = (total - raw as f64) / count as f64;
        let index = overhead - 8.0;
        
        println!("{:<12} {:>10.1} {:>10.1} {:>10.1} {:>12.0} {:>10.0} {}", 
                 "InlineHot", total/1e6, overhead, index,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
        
        let actual = map.memory_usage_actual();
        let internal_overhead = (actual - raw - count * 8) as f64 / count as f64;
        println!("  └─ actual: {:.1} MB, internal index: {:.1} B/K", 
                 actual as f64/1e6, internal_overhead);
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
        map.shrink_to_fit();
        let after = get_allocated();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_time = start.elapsed();
        
        let total = (after - before) as f64;
        let overhead = (total - raw as f64) / count as f64;
        let index = overhead - 8.0;
        
        println!("{:<12} {:>10.1} {:>10.1} {:>10.1} {:>12.0} {:>10.0} {}", 
                 "HOT", total/1e6, overhead, index,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
    }
    
    // FastArt
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
        
        let total = (after - before) as f64;
        let overhead = (total - raw as f64) / count as f64;
        let index = overhead - 8.0;
        
        println!("{:<12} {:>10.1} {:>10.1} {:>10.1} {:>12.0} {:>10.0} {}", 
                 "FastArt", total/1e6, overhead, index,
                 count as f64 / insert_time.as_secs_f64(),
                 count as f64 / lookup_time.as_secs_f64(),
                 if found == count { "✓" } else { "✗" });
    }
    
    println!("{}", "─".repeat(70));
    println!("\n✓ Index B/K = overhead - 8 (value size)");
    println!("  HOT paper target: 11-14 B/K (which includes 8-byte values = 3-6 structure)");
}
