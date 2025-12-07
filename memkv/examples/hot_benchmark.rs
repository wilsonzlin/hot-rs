//! Benchmark HOT memory overhead

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

use std::time::Instant;

fn main() {
    for count in [10_000, 100_000, 1_000_000] {
        println!("\n=== {} keys ===", count);
        
        // Generate random keys
        let keys: Vec<String> = (0..count)
            .map(|i| format!("key:{:08x}:{}", hash(i), i % 100))
            .collect();
        
        let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
        let avg = raw_key_bytes as f64 / count as f64;
        println!("Raw key bytes: {} ({:.1} avg)", raw_key_bytes, avg);
        
        // HOT
        {
            use memkv::HOT;
            
            let before = get_allocated();
            let mut t = HOT::new();
            let start = Instant::now();
            for (i, key) in keys.iter().enumerate() {
                t.insert(key.as_bytes(), i as u64);
            }
            let insert_time = start.elapsed();
            let after = get_allocated();
            
            let start = Instant::now();
            let mut found = 0;
            for (i, key) in keys.iter().enumerate() {
                if t.get(key.as_bytes()) == Some(i as u64) { found += 1; }
            }
            let lookup_time = start.elapsed();
            
            let mem = after - before;
            let overhead = mem as f64 - raw_key_bytes as f64;
            let overhead_per_key = overhead / count as f64;
            let correct = found == count;
            
            println!("HOT:       {:>7.1} MB, {:>+6.1} B/K overhead, {:>7.0} ins/s, {:>7.0} get/s {}",
                     mem as f64 / 1e6, overhead_per_key,
                     count as f64 / insert_time.as_secs_f64(),
                     count as f64 / lookup_time.as_secs_f64(),
                     if correct { "✓" } else { "✗" });
        }
        
        // FastArt for comparison
        {
            use memkv::FastArt;
            
            let before = get_allocated();
            let mut t = FastArt::new();
            let start = Instant::now();
            for (i, key) in keys.iter().enumerate() {
                t.insert(key.as_bytes(), i as u64);
            }
            let insert_time = start.elapsed();
            let after = get_allocated();
            
            let start = Instant::now();
            let mut found = 0;
            for (i, key) in keys.iter().enumerate() {
                if t.get(key.as_bytes()) == Some(i as u64) { found += 1; }
            }
            let lookup_time = start.elapsed();
            
            let mem = after - before;
            let overhead = mem as f64 - raw_key_bytes as f64;
            let overhead_per_key = overhead / count as f64;
            let correct = found == count;
            
            println!("FastArt:   {:>7.1} MB, {:>+6.1} B/K overhead, {:>7.0} ins/s, {:>7.0} get/s {}",
                     mem as f64 / 1e6, overhead_per_key,
                     count as f64 / insert_time.as_secs_f64(),
                     count as f64 / lookup_time.as_secs_f64(),
                     if correct { "✓" } else { "✗" });
        }
        
        // BTreeMap for comparison
        {
            use std::collections::BTreeMap;
            
            let before = get_allocated();
            let mut t: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
            let start = Instant::now();
            for (i, key) in keys.iter().enumerate() {
                t.insert(key.as_bytes().to_vec(), i as u64);
            }
            let insert_time = start.elapsed();
            let after = get_allocated();
            
            let start = Instant::now();
            let mut found = 0;
            for (i, key) in keys.iter().enumerate() {
                if t.get(key.as_bytes()) == Some(&(i as u64)) { found += 1; }
            }
            let lookup_time = start.elapsed();
            
            let mem = after - before;
            let overhead = mem as f64 - raw_key_bytes as f64;
            let overhead_per_key = overhead / count as f64;
            let correct = found == count;
            
            println!("BTreeMap:  {:>7.1} MB, {:>+6.1} B/K overhead, {:>7.0} ins/s, {:>7.0} get/s {}",
                     mem as f64 / 1e6, overhead_per_key,
                     count as f64 / insert_time.as_secs_f64(),
                     count as f64 / lookup_time.as_secs_f64(),
                     if correct { "✓" } else { "✗" });
        }
    }
    
    println!("\nTarget: 10-14 bytes/key overhead (HOT paper)");
}

fn hash(x: usize) -> u64 {
    let mut v = x as u64;
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51afd7ed558ccd);
    v ^= v >> 33;
    v
}

// Add MinimalSorted test
fn test_minimal_sorted(keys: &[String], count: usize, raw_key_bytes: usize) {
    use memkv::MinimalSorted;
    
    // Need to sort first
    let mut sorted: Vec<_> = keys.iter().enumerate().collect();
    sorted.sort_by(|a, b| a.1.cmp(b.1));
    
    let before = get_allocated();
    let mut t = MinimalSorted::new();
    let start = Instant::now();
    for (i, key) in &sorted {
        t.push(key.as_bytes());
    }
    let insert_time = start.elapsed();
    let after = get_allocated();
    
    // Note: MinimalSorted uses implicit values (index = value), not u64
    let start = Instant::now();
    let mut found = 0;
    for key in keys.iter() {
        if t.get(key.as_bytes()).is_some() { found += 1; }
    }
    let lookup_time = start.elapsed();
    
    let mem = after - before;
    // For MinimalSorted, overhead = mem - raw_keys (no separate value storage)
    let overhead = mem as f64 - raw_key_bytes as f64;
    let overhead_per_key = overhead / count as f64;
    let correct = found == count;
    
    println!("MinSorted: {:>7.1} MB, {:>+6.1} B/K overhead, {:>7.0} ins/s, {:>7.0} get/s {} (sorted, implicit vals)",
             mem as f64 / 1e6, overhead_per_key,
             count as f64 / insert_time.as_secs_f64(),
             count as f64 / lookup_time.as_secs_f64(),
             if correct { "✓" } else { "✗" });
}
