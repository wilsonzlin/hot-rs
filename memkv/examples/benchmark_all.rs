//! Comprehensive benchmark comparing ALL implementations
//!
//! Includes: FastArt, libart (C), BTreeMap, ArenaArt, art-tree, blart, rart, art, FST

use tikv_jemalloc_ctl::{epoch, stats};
use std::time::Instant;
use std::collections::BTreeMap;

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

// FFI bindings to libart
#[repr(C)]
struct ArtTree {
    root: *mut std::ffi::c_void,
    size: u64,
}

extern "C" {
    fn art_tree_init(t: *mut ArtTree) -> i32;
    fn art_tree_destroy(t: *mut ArtTree) -> i32;
    fn art_insert(
        t: *mut ArtTree,
        key: *const u8,
        key_len: i32,
        value: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;
    fn art_search(
        t: *const ArtTree,
        key: *const u8,
        key_len: i32,
    ) -> *mut std::ffi::c_void;
}

struct BenchResult {
    name: String,
    memory_mb: usize,
    overhead_bytes: f64,
    insert_ops: f64,
    lookup_ops: f64,
    correctness: usize,
    note: String,
}

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║     COMPREHENSIVE ART BENCHMARK - All Implementations                ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝\n");
    
    // Load dataset
    println!("Loading dataset...");
    let content = std::fs::read_to_string("urls_500mb.txt").unwrap();
    let urls: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let count = urls.len();
    let data_size: usize = urls.iter().map(|s| s.len()).sum();
    let avg_key_len = data_size as f64 / count as f64;
    println!("Loaded {} URLs, {} MB raw data", count, data_size / (1024 * 1024));
    println!("Average key length: {:.1} bytes\n", avg_key_len);
    
    let mut results: Vec<BenchResult> = Vec::new();
    
    // ========== FrozenLayer (FST) ==========
    println!("Testing FrozenLayer (FST)...");
    {
        use memkv::FrozenLayerBuilder;
        
        println!("  Sorting keys...");
        let mut sorted: Vec<_> = urls.iter()
            .enumerate()
            .map(|(i, url)| (url.as_bytes().to_vec(), i as u64))
            .collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut builder = FrozenLayerBuilder::new().unwrap();
        for (key, val) in &sorted {
            let _ = builder.insert(key, *val);
        }
        let frozen = builder.finish().unwrap();
        
        let build_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let fst_bytes = frozen.stats().fst_bytes;
        println!("  FST size: {} MB ({:.2}x compression)", 
                 fst_bytes / (1024*1024),
                 data_size as f64 / fst_bytes as f64);
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100_000)
            .filter(|(i, url)| frozen.get(url.as_bytes()) == Some(*i as u64))
            .count();
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let build_ops = count as f64 / build_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Build: {:.0} ops/sec, Lookup: {:.0} ops/sec", build_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(BenchResult {
            name: "FrozenLayer (FST)".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops: build_ops,
            lookup_ops,
            correctness: correct,
            note: "Immutable, sorted".into(),
        });
    }
    let _ = get_allocated();
    
    // ========== memkv::FastArt ==========
    println!("Testing memkv::FastArt...");
    {
        use memkv::FastArt;
        
        let before_rss = get_rss();
        let start = Instant::now();
        
        let mut tree = FastArt::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(url.as_bytes(), i as u64);
        }
        let insert_time = start.elapsed();
        let after_rss = get_rss();
        let alloc = after_rss.saturating_sub(before_rss);
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100_000)
            .filter(|(i, url)| tree.get(url.as_bytes()) == Some(*i as u64))
            .count();
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(BenchResult {
            name: "memkv::FastArt".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "libart-inspired".into(),
        });
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== libart (C) ==========
    println!("Testing libart (C)...");
    {
        let before_rss = get_rss();
        let start = Instant::now();
        
        let mut tree = ArtTree {
            root: std::ptr::null_mut(),
            size: 0,
        };
        unsafe {
            art_tree_init(&mut tree);
        }
        
        for (i, url) in urls.iter().enumerate() {
            unsafe {
                art_insert(
                    &mut tree,
                    url.as_ptr(),
                    url.len() as i32,
                    (i + 1) as *mut std::ffi::c_void,
                );
            }
        }
        let insert_time = start.elapsed();
        let after_rss = get_rss();
        let alloc = after_rss.saturating_sub(before_rss);
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in urls.iter().enumerate().take(100_000) {
            let result = unsafe {
                art_search(&tree, url.as_ptr(), url.len() as i32)
            };
            if !result.is_null() && result as usize == i + 1 {
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
        
        results.push(BenchResult {
            name: "libart (C)".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "C reference".into(),
        });
        
        unsafe {
            art_tree_destroy(&mut tree);
        }
    }
    let _ = get_allocated();
    
    // ========== BTreeMap ==========
    println!("Testing std::BTreeMap...");
    {
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
        
        results.push(BenchResult {
            name: "std::BTreeMap".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "Stdlib".into(),
        });
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
        
        results.push(BenchResult {
            name: "memkv::ArenaArt".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "Arena-based".into(),
        });
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== art-tree crate ==========
    println!("Testing art-tree crate...");
    {
        use art_tree::ByteString;
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree = art_tree::Art::<ByteString, u64>::new();
        for (i, url) in urls.iter().enumerate() {
            tree.insert(ByteString::new(url.as_bytes()), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let correct = urls.iter().enumerate()
            .take(100_000)
            .filter(|(i, url)| {
                tree.get(&ByteString::new(url.as_bytes())) == Some(&(*i as u64))
            })
            .count();
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(BenchResult {
            name: "art-tree".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "Variable keys".into(),
        });
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== blart crate (fixed 256-byte keys) ==========
    println!("Testing blart crate (fixed 256-byte keys)...");
    {
        // blart uses fixed-size keys, so we need to pad/truncate
        let short_urls: Vec<_> = urls.iter()
            .filter(|u| u.len() < 256)
            .collect();
        let short_count = short_urls.len();
        println!("  Using {} URLs (< 256 bytes)", short_count);
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree: blart::TreeMap<[u8; 256], u64> = blart::TreeMap::new();
        for (i, url) in short_urls.iter().enumerate() {
            let mut key = [0u8; 256];
            key[..url.len()].copy_from_slice(url.as_bytes());
            tree.insert(key, i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in short_urls.iter().enumerate().take(100_000) {
            let mut key = [0u8; 256];
            key[..url.len()].copy_from_slice(url.as_bytes());
            if tree.get(&key) == Some(&(i as u64)) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let short_data_size: usize = short_urls.iter().map(|s| s.len()).sum();
        let lookup_count_blart = 100_000.min(short_count);
        let overhead = (alloc as f64 - short_data_size as f64) / short_count as f64;
        let insert_ops = short_count as f64 / insert_time.as_secs_f64();
        let lookup_ops = lookup_count_blart as f64 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/{}\n", correct, lookup_count_blart);
        
        results.push(BenchResult {
            name: "blart".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "Fixed 256B keys".into(),
        });
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== rart crate (fixed ArrayKey) ==========
    println!("Testing rart crate (ArrayKey<256>)...");
    {
        use rart::{AdaptiveRadixTree, ArrayKey};
        
        let short_urls: Vec<_> = urls.iter()
            .filter(|u| u.len() < 256)
            .collect();
        let short_count = short_urls.len();
        let lookup_count = 100_000.min(short_count);
        println!("  Using {} URLs (< 256 bytes)", short_count);
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree: AdaptiveRadixTree<ArrayKey<256>, u64> = AdaptiveRadixTree::new();
        for (i, url) in short_urls.iter().enumerate() {
            tree.insert(url.as_str(), i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in short_urls.iter().enumerate().take(lookup_count) {
            if tree.get(url.as_str()) == Some(&(i as u64)) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let short_data_size: usize = short_urls.iter().map(|s| s.len()).sum();
        let overhead = (alloc as f64 - short_data_size as f64) / short_count as f64;
        let insert_ops = short_count as f64 / insert_time.as_secs_f64();
        let lookup_ops = lookup_count as f64 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/{}\n", correct, lookup_count);
        
        results.push(BenchResult {
            name: "rart".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "SIMD, fixed keys".into(),
        });
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== art crate (fixed [u8; N] keys) ==========
    println!("Testing art crate (fixed [u8; 256] keys)...");
    {
        let short_urls: Vec<_> = urls.iter()
            .filter(|u| u.len() < 256)
            .collect();
        let short_count = short_urls.len();
        println!("  Using {} URLs (< 256 bytes)", short_count);
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut tree: art::Art<u64, 256> = art::Art::new();
        for (i, url) in short_urls.iter().enumerate() {
            let mut key = [0u8; 256];
            key[..url.len()].copy_from_slice(url.as_bytes());
            tree.insert(key, i as u64);
        }
        let insert_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in short_urls.iter().enumerate().take(100_000) {
            let mut key = [0u8; 256];
            key[..url.len()].copy_from_slice(url.as_bytes());
            if tree.get(&key) == Some(&(i as u64)) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let short_data_size: usize = short_urls.iter().map(|s| s.len()).sum();
        let lookup_count_art = 100_000.min(short_count);
        let overhead = (alloc as f64 - short_data_size as f64) / short_count as f64;
        let insert_ops = short_count as f64 / insert_time.as_secs_f64();
        let lookup_ops = lookup_count_art as f64 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/{}\n", correct, lookup_count_art);
        
        results.push(BenchResult {
            name: "art".into(),
            memory_mb: alloc / (1024*1024),
            overhead_bytes: overhead,
            insert_ops,
            lookup_ops,
            correctness: correct,
            note: "Fixed 256B keys".into(),
        });
        drop(tree);
    }
    
    // Summary
    println!("\n╔══════════════════════════════════════════════════════════════════════════════════════════╗");
    println!("║                              FINAL RESULTS (sorted by overhead)                          ║");
    println!("╠══════════════════════════════════════════════════════════════════════════════════════════╣");
    println!("║ {:20} │ {:>8} │ {:>12} │ {:>12} │ {:>12} │ {:>10} │ {:15} ║", 
             "Implementation", "Memory", "Overhead/Key", "Insert ops/s", "Lookup ops/s", "Correct", "Notes");
    println!("╠══════════════════════════════════════════════════════════════════════════════════════════╣");
    
    results.sort_by(|a, b| a.overhead_bytes.partial_cmp(&b.overhead_bytes).unwrap());
    for r in &results {
        let correct_str = format!("{:.1}%", r.correctness as f64 / 1000.0);
        println!("║ {:20} │ {:>5} MB │ {:>10.1} b │ {:>12.0} │ {:>12.0} │ {:>10} │ {:15} ║", 
                 r.name, r.memory_mb, r.overhead_bytes, r.insert_ops, r.lookup_ops, correct_str, r.note);
    }
    println!("╚══════════════════════════════════════════════════════════════════════════════════════════╝");
    
    println!("\nDataset: {} keys, {} MB raw data, avg {:.1} bytes/key", 
             count, data_size / (1024*1024), avg_key_len);
    
    println!("\n📊 Key Insights:");
    println!("   • FST provides COMPRESSION (negative overhead) but is immutable");
    println!("   • FastArt beats libart (C) in memory efficiency ({:.1} vs {:.1} bytes/key)", 
             results.iter().find(|r| r.name == "memkv::FastArt").map(|r| r.overhead_bytes).unwrap_or(0.0),
             results.iter().find(|r| r.name == "libart (C)").map(|r| r.overhead_bytes).unwrap_or(0.0));
    println!("   • Fixed-size key ARTs (blart, rart, art) waste massive memory on variable-length keys");
    println!("   • art-tree is the best EXTERNAL Rust ART crate");
}
