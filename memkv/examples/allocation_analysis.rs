//! Analyze allocation overhead

use tikv_jemalloc_ctl::{epoch, stats};

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn get_allocated() -> usize {
    epoch::advance().unwrap();
    stats::allocated::read().unwrap()
}

fn main() {
    use memkv::UltraCompactArt;
    
    println!("=== Allocation Analysis ===\n");
    
    // Node size
    let node_size = std::mem::size_of::<memkv::UltraNode<u64>>();
    println!("UltraNode<u64> stack size: {} bytes", node_size);
    
    // Test allocation overhead for Box<UltraNode>
    let before = get_allocated();
    let nodes: Vec<Box<memkv::UltraNode<u64>>> = (0..10000)
        .map(|_| Box::new(memkv::UltraNode::<u64>::new_node4()))
        .collect();
    let after = get_allocated();
    let per_box = (after - before) / 10000;
    println!("Actual bytes per Box<UltraNode<u64>>: {} bytes", per_box);
    println!("  Overhead vs stack size: {} bytes ({:.1}%)", 
             per_box - node_size, 100.0 * (per_box - node_size) as f64 / node_size as f64);
    drop(nodes);
    
    // With 1.46M nodes, this is:
    let total_nodes = 1_458_050usize;
    let node_overhead = total_nodes * per_box;
    println!("\nWith {} nodes:", total_nodes);
    println!("  Pure node memory: {} MB", total_nodes * node_size / (1024 * 1024));
    println!("  Actual allocated: {} MB", node_overhead / (1024 * 1024));
    println!("  Allocation overhead: {} MB", (node_overhead - total_nodes * node_size) / (1024 * 1024));
    
    // Arena would be:
    println!("\nWith arena-based nodes:");
    println!("  Arena size: {} MB (no per-allocation overhead)", total_nodes * node_size / (1024 * 1024));
    
    // Test Vec overhead for children
    println!("\n=== Vec Overhead ===");
    let before = get_allocated();
    let vecs: Vec<Vec<Box<memkv::UltraNode<u64>>>> = (0..10000)
        .map(|_| Vec::<Box<memkv::UltraNode<u64>>>::with_capacity(4))
        .collect();
    let after = get_allocated();
    println!("Vec<Box<Node>> with capacity 4: {} bytes each", (after - before) / 10000);
    drop(vecs);
}
