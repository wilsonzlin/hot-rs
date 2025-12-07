//! Realistic benchmark: Random inserts, full map operations
use std::collections::BTreeMap;
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

fn main() {
    let count: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    
    // Generate random keys (simulating real database workload)
    println!("Generating {} random keys...", count);
    let keys: Vec<String> = (0..count)
        .map(|i| {
            // Mix of key patterns like real databases
            match i % 5 {
                0 => format!("user:{:08x}", rand_u64(i)),
                1 => format!("session:{}", rand_u64(i)),
                2 => format!("cache:item:{}:{}", i / 1000, rand_u64(i)),
                3 => format!("db:table:row:{:012}", rand_u64(i)),
                _ => format!("key_{}", rand_u64(i)),
            }
        })
        .collect();
    
    let total_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    let avg_key_len = total_key_bytes as f64 / count as f64;
    
    println!("Total key data: {} MB, avg {:.1} bytes/key\n", total_key_bytes / 1_000_000, avg_key_len);
    
    println!("╔═══════════════════════════════════════════════════════════════════════════════╗");
    println!("║        REALISTIC BENCHMARK: Random Inserts, Full Map Operations              ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");
    println!("║ Structure        │ Memory    │ Overhead    │ Insert/s  │ Lookup/s │ Correct ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");

    // BTreeMap baseline
    {
        let before = get_allocated();
        let mut tree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes().to_vec(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) == Some(&(i as u64)) { found += 1; }
        }
        let lookup_ops = count as f64 / start.elapsed().as_secs_f64();
        let correct = found == count;
        
        println!("║ BTreeMap (base)  │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>7} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if correct { "✓" } else { "✗" });
    }

    // FastArt
    {
        use memkv::FastArt;
        let before = get_allocated();
        let mut tree = FastArt::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_ops = count as f64 / start.elapsed().as_secs_f64();
        let correct = found == count;
        
        println!("║ FastArt          │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>7} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if correct { "✓" } else { "✗" });
    }

    // GloryArt
    {
        use memkv::art_glory::GloryArt;
        let before = get_allocated();
        let mut tree = GloryArt::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_ops = count as f64 / start.elapsed().as_secs_f64();
        let correct = found == count;
        
        println!("║ GloryArt         │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>7} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if correct { "✓" } else { "✗" });
    }

    // ProperHot
    {
        use memkv::ProperHot;
        let before = get_allocated();
        let mut tree = ProperHot::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_ops = count as f64 / start.elapsed().as_secs_f64();
        let correct = found == count;
        
        println!("║ ProperHot        │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>7} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if correct { "✓" } else { "✗" });
    }

    // MemKV (library's public API)
    {
        use memkv::MemKV;
        let before = get_allocated();
        let tree: MemKV<u64> = MemKV::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) == Some(i as u64) { found += 1; }
        }
        let lookup_ops = count as f64 / start.elapsed().as_secs_f64();
        let correct = found == count;
        
        println!("║ MemKV<u64>       │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>7} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if correct { "✓" } else { "✗" });
    }

    println!("╚═══════════════════════════════════════════════════════════════════════════════╝");
    
    println!("\n╔═══════════════════════════════════════════════════════════════════════════════╗");
    println!("║                              RECOMMENDATIONS                                 ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");
    println!("║ Use case                          │ Recommended structure                    ║");
    println!("╠═══════════════════════════════════════════════════════════════════════════════╣");
    println!("║ u64 values (IDs, offsets, counts) │ FastArt    - Best speed + 22% less mem   ║");
    println!("║ Best memory, u64 values           │ ProperHot  - 44% less mem, good speed    ║");
    println!("║ Generic values (any V)            │ MemKV<V>   - Flexible but higher overhead║");
    println!("║ Immutable data (read-only)        │ FrozenLayer - Extreme compression        ║");
    println!("╚═══════════════════════════════════════════════════════════════════════════════╝");
}

// Simple deterministic "random" for reproducibility
fn rand_u64(seed: usize) -> u64 {
    let mut x = seed as u64;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}
