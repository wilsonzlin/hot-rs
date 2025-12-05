//! Quick memory efficiency test for current implementations
//!
//! Run with: cargo run --release --example memory_test

use tikv_jemalloc_ctl::{epoch, stats};

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn get_rss() -> usize {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let parts: Vec<&str> = statm.split_whitespace().collect();
    if parts.len() >= 2 {
        let rss_pages: usize = parts[1].parse().unwrap_or(0);
        rss_pages * 4096
    } else {
        0
    }
}

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║           MEMORY EFFICIENCY TEST - Current Implementations           ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");
    
    // Generate test data: URLs and path-like keys
    let count = 1_000_000;
    println!("Generating {} keys...", count);
    
    let keys: Vec<String> = (0..count)
        .map(|i| {
            let domain = match i % 10 {
                0 => "example.com",
                1 => "test.org",
                2 => "domain.net",
                3 => "website.io",
                4 => "mysite.co",
                5 => "data.example.com",
                6 => "api.test.org",
                7 => "cdn.domain.net",
                8 => "static.website.io",
                _ => "files.mysite.co",
            };
            format!("https://{}/path/to/resource/{}/item/{}", domain, i / 1000, i)
        })
        .collect();
    
    let data_size: usize = keys.iter().map(|s| s.len()).sum();
    let avg_key_len = data_size as f64 / count as f64;
    
    println!("Generated {} keys, {} MB raw data, avg {:.1} bytes/key\n", 
             count, data_size / (1024 * 1024), avg_key_len);
    
    let baseline = get_allocated();
    
    // ========== FastArt ==========
    println!("Testing FastArt...");
    {
        use memkv::FastArt;
        
        let before_rss = get_rss();
        let mut tree = FastArt::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let after_rss = get_rss();
        let alloc = after_rss.saturating_sub(before_rss);
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| tree.get(key.as_bytes()) == Some(*i as u64))
            .count();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Correctness: {}/10000\n", correct);
        drop(tree);
    }
    
    // ========== HotArt ==========
    println!("Testing HotArt (HOT-inspired)...");
    {
        use memkv::HotArt;
        
        let before_rss = get_rss();
        let mut tree = HotArt::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let after_rss = get_rss();
        let alloc = after_rss.saturating_sub(before_rss);
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| tree.get(key.as_bytes()) == Some(*i as u64))
            .count();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Correctness: {}/10000\n", correct);
        drop(tree);
    }
    
    // ========== UltraArt ==========
    println!("Testing UltraArt...");
    {
        use memkv::art_ultra::UltraArt;
        
        let before = get_allocated();
        let mut tree = UltraArt::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let after = get_allocated();
        let alloc = after - before;
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| tree.get(key.as_bytes()) == Some(*i as u64))
            .count();
        
        let stats = tree.memory_usage();
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Nodes: {} N4, {} N16, {} N48, {} N256", 
                 stats.node4_count, stats.node16_count, 
                 stats.node48_count, stats.node256_count);
        println!("  Correctness: {}/10000\n", correct);
        drop(tree);
    }
    
    // ========== LeanArt ==========
    println!("Testing LeanArt...");
    {
        use memkv::art_lean::LeanArt;
        
        let before = get_allocated();
        let mut tree: LeanArt<u64> = LeanArt::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let after = get_allocated();
        let alloc = after - before;
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| tree.get(key.as_bytes()) == Some(&(*i as u64)))
            .count();
        
        let stats = tree.stats();
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Nodes: {}, Key arena: {} MB", stats.node_count, stats.key_bytes / (1024*1024));
        println!("  Correctness: {}/10000\n", correct);
        drop(tree);
    }
    
    // ========== FrontCodedIndex ==========
    println!("Testing FrontCodedIndex (prefix compression)...");
    {
        use memkv::FrontCodedBuilder;
        
        // Sort keys for front coding
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut builder = FrontCodedBuilder::new();
        for (i, key) in &sorted {
            builder.add(key.as_bytes(), *i as u64);
        }
        let front_coded = builder.finish();
        let after = get_allocated();
        let alloc = after - before;
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| front_coded.get(key.as_bytes()) == Some(&(*i as u64)))
            .count();
        
        let stats = front_coded.memory_stats();
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Data bytes: {:.1} KB, values: {:.1} KB", 
                 stats.data_bytes as f64 / 1024.0, 
                 stats.values_bytes as f64 / 1024.0);
        println!("  Correctness: {}/10000\n", correct);
        drop(front_coded);
    }
    
    // ========== FrozenLayer (FST) ==========
    println!("Testing FrozenLayer (FST)...");
    {
        use memkv::FrozenLayerBuilder;
        
        // Sort keys for FST
        let mut sorted: Vec<_> = keys.iter().enumerate().collect();
        sorted.sort_by(|a, b| a.1.cmp(b.1));
        
        let before = get_allocated();
        let mut builder = FrozenLayerBuilder::new().unwrap();
        for (i, key) in &sorted {
            let _ = builder.insert(key.as_bytes(), *i as u64);
        }
        let frozen = builder.finish().unwrap();
        let after = get_allocated();
        let alloc = after - before;
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| frozen.get(key.as_bytes()) == Some(*i as u64))
            .count();
        
        let stats = frozen.stats();
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  FST bytes: {}, compression: {:.1}x", 
                 stats.fst_bytes, data_size as f64 / stats.fst_bytes as f64);
        println!("  Correctness: {}/10000\n", correct);
        drop(frozen);
    }
    
    // ========== BTreeMap baseline ==========
    println!("Testing std::BTreeMap...");
    {
        use std::collections::BTreeMap;
        
        let before = get_allocated();
        let mut tree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes().to_vec(), i as u64);
        }
        let after = get_allocated();
        let alloc = after - before;
        
        let correct = keys.iter().enumerate()
            .take(10000)
            .filter(|(i, key)| tree.get(key.as_bytes()) == Some(&(*i as u64)))
            .count();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Correctness: {}/10000\n", correct);
        drop(tree);
    }
    
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║                              SUMMARY                                  ║");
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!("║ Target: HOT achieves 11-14 bytes/key, FST achieves compression       ║");
    println!("║ Current FastArt: ~60 bytes overhead (need to reduce to ~30)          ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
}
