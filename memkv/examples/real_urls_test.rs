//! Benchmark with REAL URLs dataset
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
    // Load real URLs
    let path = std::env::args().nth(1).unwrap_or("/tmp/urls_sample.txt".to_string());
    let limit: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    
    println!("Loading {} URLs from {}...", limit, path);
    let file = File::open(&path).expect("Cannot open file");
    let reader = BufReader::new(file);
    
    let keys: Vec<String> = reader
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.is_empty() && l != "url")
        .take(limit)
        .collect();
    
    let key_count = keys.len();
    let total_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    let avg_key_len = total_key_bytes as f64 / key_count as f64;
    
    println!("Loaded {} keys, {} bytes total, avg {:.1} bytes/key\n", 
             key_count, total_key_bytes, avg_key_len);
    
    println!("╔══════════════════════════════════════════════════════════════════════════╗");
    println!("║              REAL URLs BENCHMARK ({:>7} keys)                        ║", key_count);
    println!("╠══════════════════════════════════════════════════════════════════════════╣");
    println!("║ Structure        │ Memory    │ Overhead    │ Insert/s  │ Lookup/s │ OK? ║");
    println!("╠══════════════════════════════════════════════════════════════════════════╣");

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
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ FastArt          │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
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
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ GloryArt         │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    
    // ProperHot (TRUE HOT from paper)
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
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ ProperHot (HOT)  │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // HybridIndex (FST-based)
    {
        use memkv::HybridBuilder;
        // Sort for FST
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut builder = HybridBuilder::new();
        let start = Instant::now();
        for (i, key) in &sorted {
            builder.add(key.as_bytes(), *i as u64);
        }
        let tree = builder.finish();
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ HybridIndex(FST) │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // FrozenLayer (pure FST)
    {
        use memkv::FrozenLayer;
        // Sort for FST
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let start = Instant::now();
        let tree = FrozenLayer::from_sorted_iter(sorted.iter().map(|(i, k)| (k.as_bytes(), *i as u64))).unwrap();
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ FrozenLayer(FST) │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // GLORY (sorted array)
    {
        use memkv::Glory;
        // Sort for optimal insert
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut tree = Glory::with_capacity(key_count, total_key_bytes + key_count * 2);
        let start = Instant::now();
        for (i, key) in &sorted {
            tree.insert(key.as_bytes(), *i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ GLORY (sorted)   │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // UltraCompact (varint encoding)
    {
        use memkv::UltraCompact;
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut tree = UltraCompact::with_capacity(key_count, total_key_bytes);
        let start = Instant::now();
        for (i, key) in &sorted {
            tree.insert(key.as_bytes(), *i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ UltraCompact     │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // MinimalSorted (implicit values = index)
    {
        use memkv::MinimalSorted;
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut tree = MinimalSorted::with_capacity(total_key_bytes, key_count);
        let start = Instant::now();
        for (_i, key) in &sorted {
            tree.push(key.as_bytes());
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ MinimalSorted    │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // VarintMinimal (varint key lengths)
    {
        use memkv::VarintMinimal;
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut tree = VarintMinimal::with_capacity(total_key_bytes, key_count);
        let start = Instant::now();
        for (_i, key) in &sorted {
            tree.push(key.as_bytes());
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("║ VarintMinimal    │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    // std::collections::BTreeMap baseline
    {
        use std::collections::BTreeMap;
        let before = get_allocated();
        let mut tree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes().to_vec(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let memory = after.saturating_sub(before);
        let overhead = (memory as f64 - total_key_bytes as f64) / key_count as f64;
        let insert_ops = key_count as f64 / insert_time.as_secs_f64();
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter() {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = key_count as f64 / start.elapsed().as_secs_f64();
        let ok = found == key_count;
        
        println!("╠══════════════════════════════════════════════════════════════════════════╣");
        println!("║ BTreeMap (base)  │ {:>7.1}MB │ {:>+7.1} B/K │ {:>9.0} │ {:>8.0} │ {:>3} ║",
                 memory as f64 / 1e6, overhead, insert_ops, lookup_ops, if ok { "✓" } else { "✗" });
    }
    
    println!("╚══════════════════════════════════════════════════════════════════════════╝");
    println!("\nTarget from HOT paper: 11-14 bytes/key overhead");
    println!("Raw key data: {} MB ({:.1} bytes avg)", total_key_bytes / 1_000_000, avg_key_len);
}
