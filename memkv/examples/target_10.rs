//! Testing approaches to achieve 10 bytes/key overhead

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

use std::time::Instant;

fn main() {
    let count = 1_000_000usize;
    
    // Generate random keys
    let keys: Vec<String> = (0..count)
        .map(|i| format!("key:{:08x}:{}", hash(i), i % 100))
        .collect();
    
    let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    
    println!("=== {} keys, {} raw bytes ({:.1} avg) ===\n", count, raw_key_bytes, raw_key_bytes as f64 / count as f64);
    println!("Target: 10 bytes/key overhead (not counting u64 values)\n");
    
    // Sorted approaches (baseline)
    {
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        // MinimalSorted (implicit values)
        {
            use memkv::MinimalSorted;
            let before = get_allocated();
            let mut t = MinimalSorted::new();
            for (_, key) in &sorted {
                t.push(key.as_bytes());
            }
            let after = get_allocated();
            let mem = after - before;
            let overhead = mem as f64 - raw_key_bytes as f64;
            println!("MinimalSorted (implicit val):  {:>+6.1} B/K overhead", overhead / count as f64);
        }
        
        // Compact32 (u32 values)
        {
            use memkv::Compact32;
            let before = get_allocated();
            let mut t = Compact32::new();
            for (i, key) in &sorted {
                t.insert(key.as_bytes(), *i as u32);
            }
            let after = get_allocated();
            let mem = after - before;
            // For u32 values: overhead = mem - raw_keys - 4*count (values not counted)
            let overhead = mem as f64 - raw_key_bytes as f64 - (count * 4) as f64;
            println!("Compact32 (u32 val):           {:>+6.1} B/K overhead (not counting 4B values)", overhead / count as f64);
        }
    }
    
    println!();
    
    // Mutable approaches
    println!("Mutable (random insert order):");
    
    // HOT
    {
        use memkv::HOT;
        let before = get_allocated();
        let mut t = HOT::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        let after = get_allocated();
        let mem = after - before;
        // Overhead not counting values: mem - raw_keys - 8*count
        let overhead = mem as f64 - raw_key_bytes as f64 - (count * 8) as f64;
        println!("HOT:                           {:>+6.1} B/K overhead (not counting 8B values)", overhead / count as f64);
    }
    
    // FastArt
    {
        use memkv::FastArt;
        let before = get_allocated();
        let mut t = FastArt::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        let after = get_allocated();
        let mem = after - before;
        let overhead = mem as f64 - raw_key_bytes as f64 - (count * 8) as f64;
        println!("FastArt:                       {:>+6.1} B/K overhead (not counting 8B values)", overhead / count as f64);
    }
    
    // BTreeMap
    {
        use std::collections::BTreeMap;
        let before = get_allocated();
        let mut t: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes().to_vec(), i as u64);
        }
        let after = get_allocated();
        let mem = after - before;
        let overhead = mem as f64 - raw_key_bytes as f64 - (count * 8) as f64;
        println!("BTreeMap:                      {:>+6.1} B/K overhead (not counting 8B values)", overhead / count as f64);
    }
    
    println!("\n=== SUMMARY ===");
    println!("Sorted array (Compact32):  ~6 B/K achievable (10 with values)");
    println!("Mutable tree:              ~34 B/K currently (HOT)");
    println!("Gap:                       ~28 B/K from tree structure");
}

fn hash(x: usize) -> u64 {
    let mut v = x as u64;
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51afd7ed558ccd);
    v ^= v >> 33;
    v
}
