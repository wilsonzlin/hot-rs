fn main() {
    let count = 100_000usize;
    let keys: Vec<String> = (0..count)
        .map(|i| format!("key:{:08x}:{}", hash(i), i % 100))
        .collect();
    
    let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    println!("Keys: {}, raw bytes: {} ({:.1} avg)", count, raw_key_bytes, raw_key_bytes as f64 / count as f64);
    
    // InlineHot
    {
        use memkv::InlineHot;
        let mut t = InlineHot::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        t.shrink_to_fit();
        
        let mem = t.memory_usage_actual();
        // Overhead = mem - raw_keys - values
        let overhead = (mem - raw_key_bytes - count * 8) as f64 / count as f64;
        println!("InlineHot:  {:.1} B/K overhead (excl values)", overhead);
        
        // Breakdown:
        // key_data: raw + 2*N (len prefix) + 8*N (values)
        // nodes: 10 * (N-1) for BiNodes
        println!("  Expected: len(2) + BiNode(10) = 12 B/K");
    }
    
    // HOT for comparison
    {
        use memkv::HOT;
        let mut t = HOT::new();
        for (i, key) in keys.iter().enumerate() {
            t.insert(key.as_bytes(), i as u64);
        }
        t.shrink_to_fit();
        
        let mem = t.memory_usage_actual();
        let overhead = (mem - raw_key_bytes - count * 8) as f64 / count as f64;
        println!("HOT:        {:.1} B/K overhead (excl values)", overhead);
    }
    
    println!("\nTarget: 10-14 B/K (HOT paper)");
}

fn hash(x: usize) -> u64 {
    let mut v = x as u64;
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51afd7ed558ccd);
    v ^= v >> 33;
    v
}
