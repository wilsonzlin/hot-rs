//! Final honest summary - actual memory usage

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

use std::collections::BTreeMap;

fn main() {
    let urls_raw = std::fs::read_to_string("data/urls.txt").expect("Run from memkv directory");
    let mut urls: Vec<&str> = urls_raw.lines().collect();
    
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    urls.sort_by_key(|s| {
        let mut h = DefaultHasher::new();
        s.hash(&mut h);
        h.finish()
    });
    
    let count = urls.len();
    let raw = urls.iter().map(|u| u.len()).sum::<usize>();
    
    println!("╔═══════════════════════════════════════════════════════════════════════╗");
    println!("║            MEMORY COMPARISON - {} URLs (shuffled)                ║", count);
    println!("║                 Raw key data: {:.2} MB                               ║", raw as f64/1e6);
    println!("╚═══════════════════════════════════════════════════════════════════════╝\n");
    
    let mut results: Vec<(&str, f64, f64)> = vec![];
    
    // BTreeMap
    {
        let before = get_allocated();
        let mut map: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes().to_vec(), i as u64);
        }
        let mem = (get_allocated() - before) as f64 / 1e6;
        results.push(("BTreeMap", mem, 0.0));
    }
    
    // InlineHot
    {
        let before = get_allocated();
        let mut map = memkv::InlineHot::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        map.shrink_to_fit();
        let mem = (get_allocated() - before) as f64 / 1e6;
        results.push(("InlineHot", mem, 0.0));
    }
    
    // HOT
    {
        let before = get_allocated();
        let mut map = memkv::HOT::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        map.shrink_to_fit();
        let mem = (get_allocated() - before) as f64 / 1e6;
        results.push(("HOT", mem, 0.0));
    }
    
    // FastArt
    {
        let before = get_allocated();
        let mut map = memkv::FastArt::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        let mem = (get_allocated() - before) as f64 / 1e6;
        results.push(("FastArt", mem, 0.0));
    }
    
    let baseline = results[0].1;
    for r in &mut results {
        r.2 = (1.0 - r.1 / baseline) * 100.0;
    }
    
    println!("┌────────────┬──────────────┬──────────────┐");
    println!("│ Structure  │  Memory (MB) │ vs BTreeMap  │");
    println!("├────────────┼──────────────┼──────────────┤");
    for (name, mem, savings) in &results {
        let marker = if *savings > 0.0 { "↓" } else { "" };
        println!("│ {:<10} │ {:>12.2} │ {:>+10.1}% {} │", name, mem, savings, marker);
    }
    println!("└────────────┴──────────────┴──────────────┘\n");
    
    println!("📊 ACTUAL MEMORY USAGE (jemalloc measured):");
    println!("   BTreeMap:  {:.2} MB (baseline)", results[0].1);
    println!("   InlineHot: {:.2} MB ({:.0}% LESS memory)", results[1].1, results[1].2);
    println!("   HOT:       {:.2} MB ({:.0}% less memory)", results[2].1, results[2].2);
    println!("   FastArt:   {:.2} MB ({:.0}% less memory)", results[3].1, results[3].2);
    
    println!("\n📌 Key metrics:");
    let inline_overhead = (results[1].1 * 1e6 - raw as f64) / count as f64;
    let hot_overhead = (results[2].1 * 1e6 - raw as f64) / count as f64;
    println!("   InlineHot overhead: {:.1} B/K (total - raw keys)", inline_overhead);
    println!("   HOT overhead:       {:.1} B/K (total - raw keys)", hot_overhead);
    println!();
    println!("   HOT paper reports 11-14 B/K which INCLUDES 8-byte values.");
    println!("   Our InlineHot: {:.1} - 8 = {:.1} B/K index-only", inline_overhead, inline_overhead - 8.0);
}
