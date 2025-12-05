//! Comprehensive benchmark against all ART implementations

use tikv_jemalloc_ctl::{epoch, stats};
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn main() {
    println!("=== Comprehensive ART Benchmark ===\n");
    
    // Load dataset
    println!("Loading dataset...");
    let content = std::fs::read_to_string("urls_500mb.txt").unwrap();
    let urls: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let count = urls.len();
    let data_size: usize = urls.iter().map(|s| s.len()).sum();
    println!("Loaded {} URLs, {} MB raw\n", count, data_size / (1024 * 1024));
    
    let baseline = get_allocated();
    
    // Results
    let mut results: Vec<(&str, usize, f64, f64, usize)> = Vec::new();
    
    // ========== BTreeMap (baseline) ==========
    println!("Testing BTreeMap...");
    {
        use std::collections::BTreeMap;
        let before = get_allocated();
        let start = Instant::now();
        let mut tree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes().to_vec(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100_000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("BTreeMap", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== blart ==========
    println!("Testing blart...");
    {
        use blart::TreeMap;
        
        let before = get_allocated();
        let start = Instant::now();
        // blart requires fixed-size array keys that implement NoPrefixesBytes
        let mut tree: TreeMap<[u8; 256], u64> = TreeMap::new();
        for (i, url) in urls.iter().enumerate() {
            if url.len() <= 256 {
                let mut key = [0u8; 256];
                key[..url.len()].copy_from_slice(url.as_bytes());
                tree.insert(key, i as u64);
            }
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in urls.iter().enumerate().take(100_000) {
            if url.len() <= 256 {
                let mut key = [0u8; 256];
                key[..url.len()].copy_from_slice(url.as_bytes());
                if tree.get(&key) == Some(&(i as u64)) {
                    correct += 1;
                }
            }
        }
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("blart", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    
    // ========== art-tree ==========
    println!("Testing art-tree...");
    {
        use art_tree::{Art as ArtTree, ByteString};
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree: ArtTree<ByteString, u64> = ArtTree::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(ByteString::new(url.as_bytes()), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in urls.iter().enumerate().take(100_000) {
            if tree.get(&ByteString::new(url.as_bytes())) == Some(&(i as u64)) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("art-tree", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== art ==========
    println!("Testing art (fixed key)...");
    {
        use art::Art;
        
        // art uses fixed key length, skip URLs >= 256 bytes
        let short_urls: Vec<_> = urls.iter()
            .enumerate()
            .filter(|(_, url)| url.len() < 256)
            .collect();
        let short_count = short_urls.len();
        println!("  (Skipping {} URLs >= 256 bytes)", count - short_count);
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree: Art<u64, 256> = Art::new();
        for (i, url) in &short_urls {
            let mut key = [0u8; 256];
            key[..url.len()].copy_from_slice(url.as_bytes());
            tree.insert(key, *i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in short_urls.iter().take(100_000) {
            let mut key = [0u8; 256];
            key[..url.len()].copy_from_slice(url.as_bytes());
            if tree.get(&key) == Some(&(*i as u64)) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let short_data: usize = short_urls.iter().map(|(_, u)| u.len()).sum();
        let overhead = (alloc as f64 - short_data as f64) / short_count as f64;
        let insert_ops = short_count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("art (fixed)", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== rart ==========
    println!("Testing rart (SIMD-optimized)...");
    {
        use rart::AdaptiveRadixTree;
        use rart::ArrayKey;
        
        // rart uses fixed-size ArrayKey, skip URLs >= 256 bytes
        let short_urls: Vec<_> = urls.iter()
            .enumerate()
            .filter(|(_, url)| url.len() < 256)
            .collect();
        let short_count = short_urls.len();
        println!("  (Skipping {} URLs > 256 bytes)", count - short_count);
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree: AdaptiveRadixTree<ArrayKey<256>, u64> = AdaptiveRadixTree::new();
        for (i, url) in &short_urls {
            tree.insert(url.as_str(), *i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in short_urls.iter().take(100_000) {
            if tree.get(url.as_str()) == Some(&(*i as u64)) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        // Use short_count for overhead calculation since that's what's in the tree
        let short_data: usize = short_urls.iter().map(|(_, u)| u.len()).sum();
        let overhead = (alloc as f64 - short_data as f64) / short_count as f64;
        let insert_ops = short_count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("rart", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== memkv::ArenaArt ==========
    println!("Testing memkv::ArenaArt...");
    {
        use memkv::ArenaArt;
        
        let before = get_allocated();
        let start = Instant::now();
        let mut tree: ArenaArt<u64> = ArenaArt::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100_000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(&(*i as u64)))
            .count();
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("ArenaArt", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== FST ==========
    println!("Testing FrozenLayer (FST)...");
    {
        use memkv::FrozenLayerBuilder;
        
        // Sort
        println!("  Sorting...");
        let mut sorted: Vec<(&str, u64)> = urls.iter()
            .enumerate()
            .map(|(i, u)| (u.as_str(), i as u64))
            .collect();
        sorted.sort_by_key(|(k, _)| *k);
        
        let before = get_allocated();
        let start = Instant::now();
        let mut builder = FrozenLayerBuilder::new().unwrap();
        for (url, i) in &sorted {
            builder.insert(url.as_bytes(), *i).unwrap();
        }
        let frozen = builder.finish().unwrap();
        let build_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let fst_stats = frozen.stats();
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100_000)
            .filter(|(i, url)| frozen.get(url.as_bytes()) == Some(*i as u64))
            .count();
        let lookup_time = start.elapsed();
        
        // FST has negative overhead (compression)
        let overhead = (fst_stats.fst_bytes as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / build_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  FST size: {} MB ({:.1}x compression)", 
                 fst_stats.fst_bytes / (1024*1024), 
                 data_size as f64 / fst_stats.fst_bytes as f64);
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Build: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("FST", alloc, overhead, insert_ops, correct));
    }
    
    // ========== Summary ==========
    println!("=== SUMMARY (sorted by overhead) ===");
    println!("{:<15} {:>12} {:>15} {:>15}", "Name", "Memory", "Overhead/Key", "Insert ops/s");
    println!("{}", "-".repeat(60));
    
    results.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
    
    for (name, alloc, overhead, insert_ops, _) in &results {
        println!("{:<15} {:>10} MB {:>13.1} b {:>13.0}", 
                 name, alloc / (1024*1024), overhead, insert_ops);
    }
    
    println!("\nDataset: {} keys, {} MB raw data", count, data_size / (1024*1024));
    println!("Target: <10 bytes overhead per key");
}
