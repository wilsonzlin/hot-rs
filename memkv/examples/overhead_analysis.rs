//! Analyze where overhead comes from

fn main() {
    let count = 1_000_000usize;
    let avg_key_len = 23;
    let raw_keys = count * avg_key_len;
    
    println!("=== OVERHEAD BREAKDOWN ===");
    println!("Keys: {}, avg {} bytes, raw = {} MB\n", count, avg_key_len, raw_keys / 1_000_000);
    
    println!("HOT current leaf: key_off(4) + key_len(2) + value(8) = 14 bytes/key");
    println!("  If value not counted: 6 bytes/key just for leaf");
    println!("  Plus node overhead: ~28 bytes/key (from benchmark)");
    println!("  Total non-value overhead: ~34 bytes/key\n");
    
    println!("To get to 10 bytes overhead (not counting value):");
    println!("  - Need 10 - 8 = 2 bytes for structure per key");
    println!("  - This is essentially just a sorted array!\n");
    
    println!("SORTED ARRAY approach:");
    println!("  key_data: raw keys contiguously");
    println!("  offsets: 4 bytes per key (for random access)");
    println!("  values: 8 bytes per key (not counted)");
    println!("  Overhead = 4 bytes/key (just offsets!)\n");
    
    println!("But sorted array requires O(n) insert.");
    println!("For O(log n) insert with 10 bytes overhead, need very compact tree.\n");
    
    println!("HOT PAPER approach:");
    println!("  - Compound nodes: multiple trie levels in one node");
    println!("  - Fewer nodes = less per-key overhead");
    println!("  - SIMD search within nodes");
    println!("  - Achieves 11-14 bytes by having very few nodes");
}
