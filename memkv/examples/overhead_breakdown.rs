//! Analyze exactly where overhead comes from

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

fn main() {
    let count = 1_000_000usize;
    
    // Generate keys
    let keys: Vec<String> = (0..count)
        .map(|i| format!("key:{:08x}", i))
        .collect();
    
    let raw_key_bytes: usize = keys.iter().map(|k| k.len()).sum();
    println!("Keys: {}, Raw key bytes: {} ({:.1} avg)", 
             count, raw_key_bytes, raw_key_bytes as f64 / count as f64);
    println!();
    
    // Theoretical minimum for 10 bytes overhead:
    // - Value storage: 8 bytes × N
    // - Remaining: 2 bytes × N for structure
    println!("=== TARGET: 10 bytes/key overhead ===");
    println!("If overhead = total - raw_keys:");
    println!("  Values: {} bytes ({} B/K)", count * 8, 8);
    println!("  Structure budget: {} bytes ({} B/K)", count * 2, 2);
    println!("  Total target: {} MB", (raw_key_bytes + count * 10) as f64 / 1e6);
    println!();
    
    // Test 1: Just store keys in Vec<u8> with length prefix
    {
        let before = get_allocated();
        let mut data: Vec<u8> = Vec::with_capacity(raw_key_bytes + count * 2);
        for key in &keys {
            let len = key.len() as u16;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(key.as_bytes());
        }
        let after = get_allocated();
        let overhead = (after - before) as f64 - raw_key_bytes as f64;
        println!("Keys only (2-byte length prefix): {:+.1} B/K", overhead / count as f64);
    }
    
    // Test 2: Keys + values inline
    {
        let before = get_allocated();
        let mut data: Vec<u8> = Vec::with_capacity(raw_key_bytes + count * 10);
        for (i, key) in keys.iter().enumerate() {
            let len = key.len() as u16;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(&(i as u64).to_le_bytes());
        }
        let after = get_allocated();
        let overhead = (after - before) as f64 - raw_key_bytes as f64;
        println!("Keys + values (2B len + 8B value): {:+.1} B/K", overhead / count as f64);
    }
    
    // Test 3: Keys + values + offset index for O(log n) lookup
    {
        let before = get_allocated();
        let mut data: Vec<u8> = Vec::with_capacity(raw_key_bytes + count * 10);
        let mut offsets: Vec<u32> = Vec::with_capacity(count);
        for (i, key) in keys.iter().enumerate() {
            offsets.push(data.len() as u32);
            let len = key.len() as u16;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(&(i as u64).to_le_bytes());
        }
        let after = get_allocated();
        let mem = after - before;
        let overhead = mem as f64 - raw_key_bytes as f64;
        println!("Keys + values + offsets (2B + 8B + 4B): {:+.1} B/K", overhead / count as f64);
        println!("  Data: {} bytes", data.len());
        println!("  Offsets: {} bytes", offsets.len() * 4);
    }
    
    // Test 4: Varint length encoding
    {
        let before = get_allocated();
        let mut data: Vec<u8> = Vec::with_capacity(raw_key_bytes + count * 9);
        let mut offsets: Vec<u32> = Vec::with_capacity(count);
        for (i, key) in keys.iter().enumerate() {
            offsets.push(data.len() as u32);
            // Varint encode length
            let len = key.len();
            if len < 128 {
                data.push(len as u8);
            } else {
                data.push((len as u8) | 0x80);
                data.push((len >> 7) as u8);
            }
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(&(i as u64).to_le_bytes());
        }
        let after = get_allocated();
        let overhead = (after - before) as f64 - raw_key_bytes as f64;
        println!("Varint len + values + offsets: {:+.1} B/K", overhead / count as f64);
    }
    
    // Test 5: Implicit values (index = value)
    {
        let before = get_allocated();
        let mut data: Vec<u8> = Vec::with_capacity(raw_key_bytes + count * 2);
        let mut offsets: Vec<u32> = Vec::with_capacity(count);
        for key in &keys {
            offsets.push(data.len() as u32);
            let len = key.len() as u16;
            data.extend_from_slice(&len.to_le_bytes());
            data.extend_from_slice(key.as_bytes());
        }
        let after = get_allocated();
        let overhead = (after - before) as f64 - raw_key_bytes as f64;
        println!("Keys + offsets (implicit values): {:+.1} B/K", overhead / count as f64);
    }
    
    // Test 6: What if we could have 2-byte offsets?
    {
        // Not possible for > 64KB data, but shows the limit
        let before = get_allocated();
        let mut data: Vec<u8> = Vec::with_capacity(raw_key_bytes + count);
        for key in &keys {
            // 1-byte length (keys are < 256 bytes)
            data.push(key.len() as u8);
            data.extend_from_slice(key.as_bytes());
        }
        let after = get_allocated();
        let overhead = (after - before) as f64 - raw_key_bytes as f64;
        println!("Minimal: 1B len only (no random access): {:+.1} B/K", overhead / count as f64);
    }
    
    println!();
    println!("=== CONCLUSION ===");
    println!("For sorted data with O(log n) lookup:");
    println!("  Minimum overhead ≈ 13-14 B/K (varint + offset + value)");
    println!("For random inserts, need tree/hash structure → more overhead");
}
EOF
cargo run --release --example overhead_breakdown 2>&1 | tail -40