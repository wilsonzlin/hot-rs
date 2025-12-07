//! Analyze HOT memory breakdown

fn main() {
    let count = 100_000usize;
    let keys: Vec<String> = (0..count)
        .map(|i| format!("key:{:08x}:{}", hash(i), i % 100))
        .collect();
    
    let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    println!("Keys: {}, raw bytes: {} ({:.1} avg)", count, raw_key_bytes, raw_key_bytes as f64 / count as f64);
    
    use memkv::HOT;
    let mut t = HOT::new();
    for (i, key) in keys.iter().enumerate() {
        t.insert(key.as_bytes(), i as u64);
    }
    
    let mem_cap = t.memory_usage();
    let mem_actual = t.memory_usage_actual();
    
    println!("\nCapacity-based: {} bytes ({:.1} B/K overhead excl values)", 
             mem_cap, (mem_cap - raw_key_bytes - count * 8) as f64 / count as f64);
    println!("Actual-based:   {} bytes ({:.1} B/K overhead excl values)", 
             mem_actual, (mem_actual - raw_key_bytes - count * 8) as f64 / count as f64);
    
    // Shrink and check again
    t.shrink_to_fit();
    let mem_shrunk = t.memory_usage();
    println!("After shrink:   {} bytes ({:.1} B/K overhead excl values)", 
             mem_shrunk, (mem_shrunk - raw_key_bytes - count * 8) as f64 / count as f64);
    
    // Calculate theoretical minimum
    let leaf_size = 14; // key_off(4) + key_len(2) + value(8)
    let binode_size = 10;
    let theoretical = count * leaf_size + (count - 1) * binode_size;
    println!("\nTheoretical minimum: {} bytes ({:.1} B/K overhead excl values)", 
             theoretical + raw_key_bytes, (theoretical - count * 8) as f64 / count as f64);
}

fn hash(x: usize) -> u64 {
    let mut v = x as u64;
    v ^= v >> 33;
    v = v.wrapping_mul(0xff51afd7ed558ccd);
    v ^= v >> 33;
    v
}
