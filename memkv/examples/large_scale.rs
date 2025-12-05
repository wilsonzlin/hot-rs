//! Large-scale benchmark with 10M+ keys
//! Compares against existing Rust ART implementations

use tikv_jemalloc_ctl::{epoch, stats};
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated_bytes() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn main() {
    // Load the 500MB dataset
    println!("Loading dataset...");
    let start = Instant::now();
    let content = std::fs::read_to_string("urls_500mb.txt").unwrap();
    let urls: Vec<&str> = content.lines().collect();
    let count = urls.len();
    let data_size: usize = urls.iter().map(|s| s.len()).sum();
    println!("Loaded {} URLs ({} MB raw) in {:?}\n", 
             count, data_size / (1024 * 1024), start.elapsed());
    
    let baseline = get_allocated_bytes();
    println!("Baseline memory: {} MB\n", baseline / (1024 * 1024));
    
    
    // ========== Test: art-tree crate ==========
    println!("\n=== Testing 'art-tree' crate ===");
    {
        use art_tree::Art as ArtTree;
        use art_tree::ByteString;
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut tree: ArtTree<ByteString, u64> = ArtTree::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(ByteString::new(url.as_bytes()), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        // Verify correctness
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(&ByteString::new(url.as_bytes())) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Memory: {} MB ({:.1} bytes/key)", 
                 alloc / (1024 * 1024), alloc as f64 / count as f64);
        println!("  Insert: {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup: {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness: {}/100000", correct);
    }
    
    // Force cleanup
    let _ = get_allocated_bytes();
    std::thread::sleep(std::time::Duration::from_millis(100));
    
    // ========== Test: BTreeMap ==========
    println!("\n=== Testing BTreeMap ===");
    {
        use std::collections::BTreeMap;
        
        let before = get_allocated_bytes();
        let start = Instant::now();
        let mut tree: BTreeMap<&[u8], u64> = BTreeMap::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        // Verify correctness
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Memory: {} MB ({:.1} bytes/key)", 
                 alloc / (1024 * 1024), alloc as f64 / count as f64);
        println!("  Insert: {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup: {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness: {}/100000", correct);
    }
    
    // Force cleanup
    let _ = get_allocated_bytes();
    std::thread::sleep(std::time::Duration::from_millis(100));
    
    // ========== Test: Our ArenaArt ==========
    println!("\n=== Testing memkv::ArenaArt ===");
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
        
        // Verify correctness
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        let stats = tree.memory_stats();
        
        println!("  Memory: {} MB ({:.1} bytes/key)", 
                 alloc / (1024 * 1024), alloc as f64 / count as f64);
        println!("  Insert: {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup: {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness: {}/100000", correct);
        println!("  Breakdown: data={} MB, nodes={} MB ({} nodes)",
                 stats.data_arena_bytes / (1024 * 1024),
                 stats.node_arena_capacity / (1024 * 1024),
                 stats.node_count);
    }
    
    // Force cleanup
    let _ = get_allocated_bytes();
    std::thread::sleep(std::time::Duration::from_millis(100));
    
    // ========== Test: FrozenLayer (FST) ==========
    println!("\n=== Testing FrozenLayer (FST) ===");
    {
        use memkv::FrozenLayerBuilder;
        
        // Need sorted data for FST
        println!("  Sorting {} URLs...", count);
        let start = Instant::now();
        let mut sorted: Vec<(&str, u64)> = urls.iter()
            .enumerate()
            .map(|(i, url)| (*url, i as u64))
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
        let insert_time = start.elapsed();
        let after = get_allocated_bytes();
        let alloc = after - before;
        
        let stats = frozen.stats();
        
        // Verify correctness
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100000)
            .filter(|(i, url)| frozen.get(url.as_bytes()) == Some(*i as u64))
            .count();
        let lookup_time = start.elapsed();
        
        println!("  Memory: {} MB ({:.1} bytes/key)", 
                 alloc / (1024 * 1024), alloc as f64 / count as f64);
        println!("  FST size: {} MB ({:.1} bytes/key)", 
                 stats.fst_bytes / (1024 * 1024), stats.bytes_per_key);
        println!("  Build: {:?} ({:.0} ops/sec)", 
                 insert_time, count as f64 / insert_time.as_secs_f64());
        println!("  Lookup: {:?} for 100K ({:.0} ops/sec)", 
                 lookup_time, 100000.0 / lookup_time.as_secs_f64());
        println!("  Correctness: {}/100000", correct);
    }
    
    println!("\n=== Summary ===");
    println!("Dataset: {} URLs, {} MB raw data", count, data_size / (1024 * 1024));
    println!("Lower bytes/key is better. Target: <10 bytes overhead.");
}
