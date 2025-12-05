//! Detailed memory breakdown analysis

use memkv::UltraCompactArt;

fn main() {
    let content = std::fs::read_to_string("urls_sample.txt").unwrap();
    let urls: Vec<&str> = content.lines().collect();
    
    let mut tree: UltraCompactArt<u64> = UltraCompactArt::new();
    for (i, url) in urls.iter().enumerate() {
        tree.insert(url.as_bytes(), i as u64);
    }
    
    let stats = tree.memory_stats();
    let count = urls.len();
    
    println!("=== Memory Breakdown ({} keys) ===\n", count);
    
    // Arena (keys + prefixes)
    let arena_bytes = stats.arena_bytes;
    println!("Arena (keys + prefixes): {} KB ({:.1} bytes/key)", 
             arena_bytes / 1024, arena_bytes as f64 / count as f64);
    
    // Calculate node memory in detail
    let node_size = std::mem::size_of::<memkv::UltraNode<u64>>();
    let total_nodes = stats.leaf_count + stats.node4_count + stats.node16_count + 
                      stats.node48_count + stats.node256_count;
    
    println!("\nNode counts:");
    println!("  Leaves: {} ({} bytes each)", stats.leaf_count, node_size);
    println!("  Node4: {} ({} bytes each + Vec<Box>)", stats.node4_count, node_size);
    println!("  Node16: {} ({} bytes each + Vec<Box>)", stats.node16_count, node_size);
    println!("  Node48: {} ({} bytes each + child_index(256) + Vec)", stats.node48_count, node_size);
    println!("  Node256: {} ({} bytes each + children array(2048))", stats.node256_count, node_size);
    
    // Estimate heap allocations for each node type
    let box_size = 8usize; // 64-bit pointer
    let vec_overhead = 24usize; // ptr, len, cap
    
    // Leaves: just the Box allocation (node_size bytes each)
    let leaf_heap = stats.leaf_count * node_size;
    
    // Node4: Box + Vec for children (avg 2-3 children)
    let node4_box = stats.node4_count * node_size;
    let node4_children = stats.node4_count * (2 * box_size); // avg 2 children
    
    // Node16: Box + Vec for children (avg 8-10 children)
    let node16_box = stats.node16_count * node_size;
    let node16_children = stats.node16_count * (10 * box_size); // avg 10 children
    
    // Node48: Box + child_index(256) + Vec (avg 30 children)
    let node48_box = stats.node48_count * node_size;
    let node48_index = stats.node48_count * 256;
    let node48_children = stats.node48_count * (30 * box_size);
    
    // Node256: Box + children array (256 * 8 = 2048)
    let node256_box = stats.node256_count * node_size;
    let node256_children = stats.node256_count * (256 * box_size);
    
    let total_node_heap = leaf_heap + node4_box + node4_children + 
                          node16_box + node16_children + 
                          node48_box + node48_index + node48_children +
                          node256_box + node256_children;
    
    println!("\nEstimated heap memory:");
    println!("  Leaf boxes: {} KB", leaf_heap / 1024);
    println!("  Node4 (box + children): {} KB", (node4_box + node4_children) / 1024);
    println!("  Node16 (box + children): {} KB", (node16_box + node16_children) / 1024);
    println!("  Node48 (box + index + children): {} KB", (node48_box + node48_index + node48_children) / 1024);
    println!("  Node256 (box + children): {} KB", (node256_box + node256_children) / 1024);
    println!("  Total node heap: {} KB", total_node_heap / 1024);
    
    println!("\nTotal estimated: {} KB ({:.1} bytes/key)", 
             (arena_bytes + total_node_heap) / 1024,
             (arena_bytes + total_node_heap) as f64 / count as f64);
    
    // Potential savings from pointer compression
    let current_ptr_overhead = (node4_children + node16_children + node48_children + node256_children);
    let compressed_ptr_overhead = current_ptr_overhead / 2; // 4 bytes instead of 8
    let ptr_savings = current_ptr_overhead - compressed_ptr_overhead;
    
    println!("\n=== Pointer Compression Opportunity ===");
    println!("  Current child pointer overhead: {} KB", current_ptr_overhead / 1024);
    println!("  With 32-bit offsets: {} KB", compressed_ptr_overhead / 1024);
    println!("  Potential savings: {} KB ({:.1}%)", 
             ptr_savings / 1024,
             100.0 * ptr_savings as f64 / (arena_bytes + total_node_heap) as f64);
}
