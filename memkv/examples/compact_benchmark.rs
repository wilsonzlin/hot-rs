//! Benchmark CompactHot memory overhead

fn main() {
    let count = 100_000usize;
    let keys: Vec<String> = (0..count)
        .map(|i| format!("key:{:08x}:{}", hash(i), i % 100))
        .collect();
    
    let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    println!("Keys: {}, raw bytes: {} ({:.1} avg)", count, raw_key_bytes, raw_key_bytes as f64 / count as f64);
    
    // CompactHot
    {
        use memkv::CompactHot;
        let mut t = CompactHot::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        t.shrink_to_fit();
        
        let mem = t.memory_usage_actual();
        println!("\nCompactHot:");
        println!("  Memory: {} bytes", mem);
        println!("  Overhead (excl raw keys): {:.1} B/K", (mem - raw_key_bytes) as f64 / count as f64);
        println!("  Overhead (excl values): {:.1} B/K", (mem - raw_key_bytes - count * 8) as f64 / count as f64);
        
        // Breakdown:
        // - key_data: raw_keys + 2 bytes len prefix per key
        // - leaves: 12 bytes each
        // - nodes: 8 bytes per BiNode, N-1 BiNodes
        let key_data_overhead = count * 2; // len prefixes
        let leaf_overhead = count * 12;
        let node_overhead = (count - 1) * 8;
        println!("\n  Breakdown:");
        println!("    Key len prefixes: {} bytes ({:.1} B/K)", key_data_overhead, key_data_overhead as f64 / count as f64);
        println!("    Leaf structs: {} bytes ({:.1} B/K)", leaf_overhead, leaf_overhead as f64 / count as f64);
        println!("    BiNodes: {} bytes ({:.1} B/K)", node_overhead, node_overhead as f64 / count as f64);
        println!("    Total excl value (8 B/K): {:.1} B/K", 
                 (key_data_overhead + leaf_overhead - count * 8 + node_overhead) as f64 / count as f64);
    }
    
    // HOT (original)
    {
        use memkv::HOT;
        let mut t = HOT::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        t.shrink_to_fit();
        
        let mem = t.memory_usage_actual();
        println!("\nHOT (original):");
        println!("  Memory: {} bytes", mem);
        println!("  Overhead (excl values): {:.1} B/K", (mem - raw_key_bytes - count * 8) as f64 / count as f64);
    }
    
}


fn hash(x: usize) -> u64 {
    let mut v = x as u64;
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51afd7ed558ccd);
    v ^= v >> 33;
    v
}
