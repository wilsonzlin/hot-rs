//! Benchmark comparing against libart (C implementation)

use tikv_jemalloc_ctl::{epoch, stats};
use std::time::Instant;

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn get_rss() -> usize {
    // Read RSS from /proc/self/statm
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let parts: Vec<&str> = statm.split_whitespace().collect();
    if parts.len() >= 2 {
        // Second field is RSS in pages
        let rss_pages: usize = parts[1].parse().unwrap_or(0);
        rss_pages * 4096 // Convert to bytes (4KB pages)
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

fn main() {
    println!("=== libart (C) vs Rust Implementations ===\n");
    
    // Load dataset
    println!("Loading dataset...");
    let content = std::fs::read_to_string("urls_500mb.txt").unwrap();
    let urls: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let count = urls.len();
    let data_size: usize = urls.iter().map(|s| s.len()).sum();
    println!("Loaded {} URLs, {} MB raw\n", count, data_size / (1024 * 1024));
    
    let mut results: Vec<(&str, usize, f64, f64, usize)> = Vec::new();
    
    // ========== libart (C) ==========
    println!("Testing libart (C)...");
    {
        // Use RSS for C library since it uses glibc malloc, not jemalloc
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
                    (i + 1) as *mut std::ffi::c_void, // +1 so NULL means not found
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
        
        println!("  Memory (RSS): {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("libart (C)", alloc, overhead, insert_ops, correct));
        
        unsafe {
            art_tree_destroy(&mut tree);
        }
    }
    let _ = get_allocated();
    
    // ========== BTreeMap ==========
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
        let mut correct = 0;
        for (i, url) in urls.iter().enumerate().take(100_000) {
            if tree.get(url.as_bytes()) == Some(&(i as u64)) {
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
        
        results.push(("ArenaArt", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== memkv::FastArt ==========
    println!("Testing memkv::FastArt...");
    {
        use memkv::FastArt;
        
        // FastArt uses system malloc, measure with RSS
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
        let mut correct = 0;
        for (i, url) in urls.iter().enumerate().take(100_000) {
            if tree.get(url.as_bytes()) == Some(i as u64) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let insert_ops = count as f64 / insert_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Insert: {:.0} ops/sec, Lookup: {:.0} ops/sec", insert_ops, lookup_ops);
        println!("  Correctness: {}/100000 (WIP)\n", correct);
        
        results.push(("FastArt (WIP)", alloc, overhead, insert_ops, correct));
        drop(tree);
    }
    let _ = get_allocated();
    
    // ========== FST ==========
    println!("Testing FrozenLayer (FST)...");
    {
        use memkv::FrozenLayerBuilder;
        
        println!("  Sorting...");
        let mut sorted: Vec<_> = urls.iter()
            .enumerate()
            .map(|(i, url)| (url.as_bytes().to_vec(), i as u64))
            .collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        
        let before = get_allocated();
        let start = Instant::now();
        
        let mut builder = FrozenLayerBuilder::new().unwrap();
        for (key, val) in &sorted {
            if let Err(_) = builder.insert(key, *val) {
                continue; // Skip duplicates
            }
        }
        let frozen = builder.finish().unwrap();
        
        let build_time = start.elapsed();
        let after = get_allocated();
        let alloc = after - before;
        
        let fst_bytes = frozen.stats().fst_bytes;
        println!("  FST size: {} MB ({}x compression)", 
                 fst_bytes / (1024*1024),
                 data_size as f64 / fst_bytes as f64);
        
        let start = Instant::now();
        let mut correct = 0;
        for (i, url) in urls.iter().enumerate().take(100_000) {
            if frozen.get(url.as_bytes()) == Some(i as u64) {
                correct += 1;
            }
        }
        let lookup_time = start.elapsed();
        
        let overhead = (alloc as f64 - data_size as f64) / count as f64;
        let build_ops = count as f64 / build_time.as_secs_f64();
        let lookup_ops = 100_000.0 / lookup_time.as_secs_f64();
        
        println!("  Memory: {} MB, {:.1} bytes overhead/key", alloc / (1024*1024), overhead);
        println!("  Build: {:.0} ops/sec, Lookup: {:.0} ops/sec", build_ops, lookup_ops);
        println!("  Correctness: {}/100000\n", correct);
        
        results.push(("FST", alloc, overhead, build_ops, correct));
    }
    
    // Summary
    println!("\n=== SUMMARY (sorted by overhead) ===");
    println!("{:<20} {:>10} {:>15} {:>15}", "Name", "Memory", "Overhead/Key", "Insert ops/s");
    println!("{:-<60}", "");
    
    results.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap());
    for (name, mem, overhead, ops, _correct) in &results {
        println!("{:<20} {:>7} MB {:>12.1} b {:>15.0}", 
                 name, mem / (1024*1024), overhead, ops);
    }
    
    println!("\nDataset: {} keys, {} MB raw data", count, data_size / (1024*1024));
}
