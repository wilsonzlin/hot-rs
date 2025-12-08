//! Basic usage examples for memkv structures.

use memkv::{InlineHot, FastArt, MemKV};

fn main() {
    println!("=== InlineHot (Best Memory Efficiency) ===\n");
    demo_inline_hot();
    
    println!("\n=== FastArt (Best Speed) ===\n");
    demo_fast_art();
    
    println!("\n=== MemKV (Thread-safe, Generic Values) ===\n");
    demo_memkv();
}

fn demo_inline_hot() {
    let mut map = InlineHot::new();
    
    // Insert
    map.insert(b"user:1001", 1001);
    map.insert(b"user:1002", 1002);
    map.insert(b"post:100", 100);
    
    // Lookup
    println!("user:1001 = {:?}", map.get(b"user:1001"));
    println!("missing   = {:?}", map.get(b"missing"));
    
    // Update
    map.insert(b"user:1001", 9999);
    println!("updated   = {:?}", map.get(b"user:1001"));
    
    // Memory stats
    println!("count     = {}", map.len());
    println!("memory    = {} bytes", map.memory_usage_actual());
}

fn demo_fast_art() {
    let mut art = FastArt::new();
    
    // Insert
    art.insert(b"user:1001", 1001);
    art.insert(b"user:1002", 1002);
    art.insert(b"post:100", 100);
    
    // Lookup
    println!("user:1001 = {:?}", art.get(b"user:1001"));
    println!("missing   = {:?}", art.get(b"missing"));
    
    // Update
    art.insert(b"user:1001", 9999);
    println!("updated   = {:?}", art.get(b"user:1001"));
    
    println!("count     = {}", art.len());
}

fn demo_memkv() {
    // Thread-safe, works with any Clone type
    let kv: MemKV<String> = MemKV::new();
    
    kv.insert(b"name", "Alice".to_string());
    kv.insert(b"city", "Boston".to_string());
    
    println!("name = {:?}", kv.get(b"name"));
    println!("city = {:?}", kv.get(b"city"));
    
    // Prefix scan
    kv.insert(b"user:1", "User1".to_string());
    kv.insert(b"user:2", "User2".to_string());
    
    println!("\nPrefix 'user:':");
    for (key, value) in kv.prefix(b"user:") {
        println!("  {} = {}", String::from_utf8_lossy(&key), value);
    }
}
