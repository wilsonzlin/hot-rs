//! Benchmark CompoundHot memory overhead

fn main() {
    for count in [10_000, 100_000] {
        let keys: Vec<String> = (0..count)
            .map(|i| format!("key:{:08x}:{}", hash(i), i % 100))
            .collect();
        
        let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
        println!("\n=== {} keys, {:.1} avg key len ===", count, raw_key_bytes as f64 / count as f64);
        
        // CompoundHot
        {
            use memkv::CompoundHot;
            let mut t = CompoundHot::new();
            for (i, key) in keys.iter().enumerate() {
                t.insert(key.as_bytes(), i as u64);
            }
            t.shrink_to_fit();
            
            let mem = t.memory_usage_actual();
            let overhead = (mem - raw_key_bytes - count * 8) as f64 / count as f64;
            println!("CompoundHot: {:.1} B/K overhead (excl values)", overhead);
        }
        
        // HOT (BiNodes)
        {
            use memkv::HOT;
            let mut t = HOT::new();
            for (i, key) in keys.iter().enumerate() {
                t.insert(key.as_bytes(), i as u64);
            }
            t.shrink_to_fit();
            
            let mem = t.memory_usage_actual();
            let overhead = (mem - raw_key_bytes - count * 8) as f64 / count as f64;
            println!("HOT (BiNodes): {:.1} B/K overhead (excl values)", overhead);
        }
        
        // CompactHot
        {
            use memkv::CompactHot;
            let mut t = CompactHot::new();
            for (i, key) in keys.iter().enumerate() {
                t.insert(key.as_bytes(), i as u64);
            }
            t.shrink_to_fit();
            
            let mem = t.memory_usage_actual();
            let overhead = (mem - raw_key_bytes - count * 8) as f64 / count as f64;
            println!("CompactHot:  {:.1} B/K overhead (excl values)", overhead);
        }
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
