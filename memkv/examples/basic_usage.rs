//! Basic usage example for MemKV.

use memkv::MemKV;

fn main() {
    // Create a new store
    let kv: MemKV<u64> = MemKV::new();

    // Insert some data
    println!("Inserting data...");
    kv.insert(b"user:1001", 1001);
    kv.insert(b"user:1002", 1002);
    kv.insert(b"user:1003", 1003);
    kv.insert(b"post:100", 100);
    kv.insert(b"post:101", 101);

    // Point lookups
    println!("\nPoint lookups:");
    println!("  user:1001 = {:?}", kv.get(b"user:1001"));
    println!("  user:9999 = {:?}", kv.get(b"user:9999"));

    // Prefix scan
    println!("\nPrefix scan for 'user:':");
    for (key, value) in kv.prefix(b"user:") {
        println!("  {} = {}", String::from_utf8_lossy(&key), value);
    }

    // Range query
    println!("\nRange query [post:100, post:102):");
    for (key, value) in kv.range(b"post:100", b"post:102") {
        println!("  {} = {}", String::from_utf8_lossy(&key), value);
    }

    // Memory stats
    let stats = kv.memory_usage();
    println!("\nMemory statistics:");
    println!("  Keys: {}", stats.num_keys);
    println!("  Key bytes: {}", stats.key_bytes);
    println!("  Node bytes: {}", stats.node_bytes);
    println!("  Bytes per key: {:.2}", stats.bytes_per_key);

    // Update and remove
    println!("\nUpdating user:1001...");
    let old = kv.insert(b"user:1001", 9999);
    println!("  Old value: {:?}", old);
    println!("  New value: {:?}", kv.get(b"user:1001"));

    println!("\nRemoving user:1002...");
    let removed = kv.remove(b"user:1002");
    println!("  Removed: {:?}", removed);
    println!("  Still exists: {}", kv.contains(b"user:1002"));

    println!("\nFinal count: {} keys", kv.len());
}
