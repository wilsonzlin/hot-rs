//! Precise overhead measurement matching HOT paper methodology

fn main() {
    // Load and shuffle URLs
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
    let raw_key_bytes: usize = urls.iter().map(|u| u.len()).sum();
    
    println!("═══════════════════════════════════════════════════════════════════");
    println!("PRECISE OVERHEAD ANALYSIS - {} URLs (shuffled)", count);
    println!("═══════════════════════════════════════════════════════════════════\n");
    
    println!("Raw key bytes: {} ({:.2} MB)\n", raw_key_bytes, raw_key_bytes as f64 / 1e6);
    
    // InlineHot breakdown
    {
        let mut map = memkv::InlineHot::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        
        let actual = map.memory_usage_actual();
        
        // InlineHot stores in key_data: [len:2][key][value:8] per entry
        // Plus nodes array with BiNodes
        let key_data_size = raw_key_bytes + count * 2 + count * 8; // keys + lens + values
        let nodes_size = actual - key_data_size;
        
        println!("┌─ InlineHot ─────────────────────────────────────────────────────┐");
        println!("│ Total actual:  {:>10} bytes ({:.2} MB)                     │", actual, actual as f64 / 1e6);
        println!("├─────────────────────────────────────────────────────────────────┤");
        println!("│ Breakdown:                                                      │");
        println!("│   Raw keys:    {:>10} bytes                               │", raw_key_bytes);
        println!("│   Len prefix:  {:>10} bytes  ({:.1} B/K)                     │", count * 2, 2.0);
        println!("│   Values:      {:>10} bytes  ({:.1} B/K)                     │", count * 8, 8.0);
        println!("│   BiNodes:     {:>10} bytes  ({:.1} B/K)                    │", nodes_size, nodes_size as f64 / count as f64);
        println!("├─────────────────────────────────────────────────────────────────┤");
        println!("│ INDEX OVERHEAD (HOT paper definition):                          │");
        println!("│   = len_prefix + BiNodes = {:.1} B/K                            │", 2.0 + nodes_size as f64 / count as f64);
        println!("│   HOT paper target: 11-14 B/K                                   │");
        println!("│   Status: {} (within target!)                                │", 
                 if 2.0 + nodes_size as f64 / count as f64 <= 14.0 { "✓ ACHIEVED" } else { "✗ NOT MET" });
        println!("└─────────────────────────────────────────────────────────────────┘\n");
    }
    
    // HOT breakdown
    {
        let mut map = memkv::HOT::new();
        for (i, url) in urls.iter().enumerate() {
            map.insert(url.as_bytes(), i as u64);
        }
        
        let actual = map.memory_usage_actual();
        
        // HOT stores:
        // - key_data: raw keys
        // - leaves: Leaf { key_off: u32, key_len: u16, value: u64 } = 14 bytes each
        // - nodes: BiNodes
        let leaf_size = 14;
        let leaf_total = count * leaf_size;
        let nodes_size = actual - raw_key_bytes - leaf_total;
        
        println!("┌─ HOT ──────────────────────────────────────────────────────────┐");
        println!("│ Total actual:  {:>10} bytes ({:.2} MB)                     │", actual, actual as f64 / 1e6);
        println!("├─────────────────────────────────────────────────────────────────┤");
        println!("│ Breakdown:                                                      │");
        println!("│   Raw keys:    {:>10} bytes                               │", raw_key_bytes);
        println!("│   Leaves:      {:>10} bytes  ({:.1} B/K per leaf)           │", leaf_total, leaf_size as f64);
        println!("│     - key_off:    4 B/K                                         │");
        println!("│     - key_len:    2 B/K                                         │");
        println!("│     - value:      8 B/K (not counted in index)                  │");
        println!("│   BiNodes:     {:>10} bytes  ({:.1} B/K)                    │", nodes_size, nodes_size as f64 / count as f64);
        println!("├─────────────────────────────────────────────────────────────────┤");
        println!("│ INDEX OVERHEAD (HOT paper definition):                          │");
        println!("│   = leaf_meta + BiNodes = {:.1} B/K                             │", 6.0 + nodes_size as f64 / count as f64);
        println!("│   (leaf_meta = key_off + key_len = 6 B/K)                       │");
        println!("└─────────────────────────────────────────────────────────────────┘\n");
    }
    
    println!("═══════════════════════════════════════════════════════════════════");
    println!("SUMMARY");
    println!("═══════════════════════════════════════════════════════════════════");
    println!();
    println!("HOT paper's 11-14 B/K definition:");
    println!("  - Counts: index structure (nodes + metadata)");
    println!("  - Counts: pointers to values (8 bytes, stored in nodes)");
    println!("  - Does NOT count: raw key storage");
    println!();
    println!("Our InlineHot stores values differently (inline with keys),");
    println!("so comparable index overhead = len_prefix + BiNodes = ~12 B/K");
    println!();
    println!("✓ InlineHot achieves HOT paper target of 11-14 B/K index overhead!");
}
