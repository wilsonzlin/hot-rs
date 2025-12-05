//! Large-scale benchmark with owned keys (fair comparison)

use tikv_jemalloc_ctl::{epoch, stats};
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated_bytes() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn main() {
    println!("=== Large-Scale Memory Benchmark (Fair: Owned Keys) ===\n");
    
    // Load dataset
    println!("Loading dataset...");
    let start = Instant::now();
    let content = std::fs::read_to_string("urls_500mb.txt").unwrap();
    let urls: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let count = urls.len();
    let data_size: usize = urls.iter().map(|s| s.len()).sum();
    let avg_key_len = data_size as f64 / count as f64;
    println!("Loaded {} URLs in {:?}", count, start.elapsed());
    println!("Raw key data: {} MB ({:.1} bytes avg/key)\n", data_size / (1024 * 1024), avg_key_len);
    
    // Clear baseline
    drop(content);
    let baseline = get_allocated_bytes();
    println!("Baseline (URLs in memory): {} MB\n", baseline / (1024 * 1024));

    // ========== BTreeMap with Vec<u8> keys ==========
    println!("=== BTreeMap<Vec<u8>, u64> ===");
    {
        use std::collections::BTreeMap;
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut tree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes().to_vec(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        let overhead = alloc as f64 - data_size as f64;
        let overhead_per_key = overhead / count as f64;
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Total memory:    {} MB", alloc / (1024 * 1024));
        println!("  Bytes/key:       {:.1}", alloc as f64 / count as f64);
        println!("  Overhead/key:    {:.1} bytes", overhead_per_key);
        println!("  Insert:          {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup:          {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness:     {}/100000\n", correct);
        
        drop(tree);
    }
    let _ = get_allocated_bytes();
    
    // ========== art-tree ==========
    println!("=== art-tree crate ===");
    {
        use art_tree::Art;
        use art_tree::ByteString;
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut tree: Art<ByteString, u64> = Art::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(ByteString::new(url.as_bytes()), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        let overhead = alloc as f64 - data_size as f64;
        let overhead_per_key = overhead / count as f64;
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(&ByteString::new(url.as_bytes())) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Total memory:    {} MB", alloc / (1024 * 1024));
        println!("  Bytes/key:       {:.1}", alloc as f64 / count as f64);
        println!("  Overhead/key:    {:.1} bytes", overhead_per_key);
        println!("  Insert:          {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup:          {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness:     {}/100000\n", correct);
        
        drop(tree);
    }
    let _ = get_allocated_bytes();
    
    // ========== memkv ArenaArt ==========
    println!("=== memkv::ArenaArt ===");
    {
        use memkv::ArenaArt;
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut tree: ArenaArt<u64> = ArenaArt::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        let overhead = alloc as f64 - data_size as f64;
        let overhead_per_key = overhead / count as f64;
        
        let stats = tree.memory_stats();
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Total memory:    {} MB", alloc / (1024 * 1024));
        println!("  Bytes/key:       {:.1}", alloc as f64 / count as f64);
        println!("  Overhead/key:    {:.1} bytes", overhead_per_key);
        println!("  Data arena:      {} MB", stats.data_arena_bytes / (1024 * 1024));
        println!("  Node arena:      {} MB ({} nodes, {} bytes/node)", 
                 stats.node_arena_capacity / (1024 * 1024),
                 stats.node_count,
                 stats.node_arena_capacity / stats.node_count);
        println!("  Insert:          {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup:          {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness:     {}/100000\n", correct);
        
        drop(tree);
    }
    let _ = get_allocated_bytes();
    
    // ========== OptimizedArt ==========
    println!("=== memkv::OptimizedArt (DuckDB-style) ===");
    {
        use memkv::OptimizedArt;
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut tree: OptimizedArt<u64> = OptimizedArt::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        let overhead = alloc as f64 - data_size as f64;
        let overhead_per_key = overhead / count as f64;
        
        let stats = tree.memory_stats();
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Total memory:    {} MB", alloc / (1024 * 1024));
        println!("  Bytes/key:       {:.1}", alloc as f64 / count as f64);
        println!("  Overhead/key:    {:.1} bytes", overhead_per_key);
        println!("  Key arena:       {} MB", stats.key_arena_bytes / (1024 * 1024));
        println!("  Node arena:      {} MB ({} nodes)", 
                 stats.node_arena_bytes / (1024 * 1024),
                 stats.node_count);
        println!("  Insert:          {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup:          {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness:     {}/100000\n", correct);
        
        drop(tree);
    }
    let _ = get_allocated_bytes();
    
    // ========== FST ==========
    println!("=== FrozenLayer (FST) ===");
    {
        use memkv::FrozenLayerBuilder;
        
        // Sort first
        println!("  Sorting {} URLs...", count);
        let start = Instant::now();
        let mut sorted: Vec<(&str, u64)> = urls.iter()
            .enumerate()
            .map(|(i, url)| (url.as_str(), i as u64))
            .collect();
        sorted.sort_by_key(|(k, _)| *k);
        println!("  Sorted in {:?}", start.elapsed());
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut builder = FrozenLayerBuilder::new().unwrap();
        for (url, i) in &sorted {
            builder.insert(url.as_bytes(), *i).unwrap();
        }
        let frozen = builder.finish().unwrap();
        let build_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        let fst_stats = frozen.stats();
        let compression = data_size as f64 / fst_stats.fst_bytes as f64;
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| frozen.get(url.as_bytes()) == Some(*i as u64))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Total memory:    {} MB (FST: {} MB)", 
                 alloc / (1024 * 1024), fst_stats.fst_bytes / (1024 * 1024));
        println!("  Bytes/key:       {:.1} (FST: {:.1})", 
                 alloc as f64 / count as f64, fst_stats.bytes_per_key);
        println!("  Compression:     {:.1}x vs raw", compression);
        println!("  Build:           {:?} ({:.0} ops/sec)", 
                 build_time, count as f64 / build_time.as_secs_f64());
        println!("  Lookup:          {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness:     {}/100000\n", correct);
    }
    
    // ========== Summary ==========
    println!("=== SUMMARY ===");
    println!("Dataset: {} keys, {} MB raw, {:.1} bytes avg key", 
             count, data_size / (1024 * 1024), avg_key_len);
    println!("\nTarget: <10 bytes OVERHEAD per key (excluding key data itself)");
    println!("Raw data must be stored somewhere - the question is how much EXTRA");
}
