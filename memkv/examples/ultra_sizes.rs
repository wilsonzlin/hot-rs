//! Check UltraNode sizes.

use memkv::art_compact2::{UltraNode, DataRef};

fn main() {
    println!("=== UltraCompactArt Node Sizes ===\n");
    
    println!("DataRef: {} bytes", std::mem::size_of::<DataRef>());
    println!("Box<UltraNode<u64>>: {} bytes", std::mem::size_of::<Box<UltraNode<u64>>>());
    println!("Option<Box<UltraNode<u64>>>: {} bytes", std::mem::size_of::<Option<Box<UltraNode<u64>>>>());
    println!("Option<(DataRef, u64)>: {} bytes", std::mem::size_of::<Option<(DataRef, u64)>>());
    
    println!("\nUltraNode<u64> enum: {} bytes", std::mem::size_of::<UltraNode<u64>>());
    
    // Create actual nodes to see heap usage
    let leaf = UltraNode::<u64>::new_leaf(DataRef::empty(), 42);
    let node4 = UltraNode::<u64>::new_node4();
    let node16 = UltraNode::<u64>::new_node16();
    let node48 = UltraNode::<u64>::new_node48();
    let node256 = UltraNode::<u64>::new_node256();
    
    println!("\n=== Actual Node Stack Sizes ===");
    println!("(These are the sizes of Box<Node> allocations)\n");
    
    // Match to show variant sizes
    match &leaf {
        UltraNode::Leaf { .. } => println!("Leaf: {} bytes", std::mem::size_of_val(&leaf)),
        _ => {}
    }
    match &node4 {
        UltraNode::Node4 { .. } => println!("Node4: {} bytes", std::mem::size_of_val(&node4)),
        _ => {}
    }
    match &node16 {
        UltraNode::Node16 { .. } => println!("Node16: {} bytes", std::mem::size_of_val(&node16)),
        _ => {}
    }
    match &node48 {
        UltraNode::Node48 { .. } => println!("Node48: {} bytes", std::mem::size_of_val(&node48)),
        _ => {}
    }
    match &node256 {
        UltraNode::Node256 { .. } => println!("Node256: {} bytes", std::mem::size_of_val(&node256)),
        _ => {}
    }
    
    // Child array sizes
    println!("\n=== Array Sizes ===");
    println!("[Option<Box<UltraNode>>; 4]: {} bytes", 
             std::mem::size_of::<[Option<Box<UltraNode<u64>>>; 4]>());
    println!("[Option<Box<UltraNode>>; 16]: {} bytes", 
             std::mem::size_of::<[Option<Box<UltraNode<u64>>>; 16]>());
    println!("Box<[u8; 256]>: {} bytes (ptr) + 256 heap", 
             std::mem::size_of::<Box<[u8; 256]>>());
    println!("Box<[Option<Box<UltraNode>>; 256]>: {} bytes (ptr) + {} heap", 
             std::mem::size_of::<Box<[Option<Box<UltraNode<u64>>>; 256]>>(),
             256 * std::mem::size_of::<Option<Box<UltraNode<u64>>>>());
    
    // Estimated real usage per node type for 967K URLs dataset
    println!("\n=== Estimated Memory for URL Dataset ===");
    let leaves = 966_956usize;
    let node4s = 438_899usize;
    let node16s = 49_465usize;
    let node48s = 2_713usize;
    let node256s = 17usize;
    
    let node_size = std::mem::size_of::<UltraNode<u64>>();
    
    println!("Each Box<UltraNode<u64>> allocation: {} bytes", node_size);
    println!("Total Box allocations: {} nodes", leaves + node4s + node16s + node48s + node256s);
    println!("Node allocation memory: {} MB", 
             (leaves + node4s + node16s + node48s + node256s) * node_size / (1024 * 1024));
    
    // Additional heap for Node48 and Node256
    let node48_extra = node48s * 256; // Box<[u8; 256]>
    let node256_extra = node256s * 256 * 8; // Box<[Option<Box>; 256]>
    println!("Node48 child_index heap: {} KB", node48_extra / 1024);
    println!("Node256 children heap: {} KB", node256_extra / 1024);
    
    // Vec overhead for Node48
    let node48_vec = node48s * 24; // Vec overhead
    println!("Node48 Vec overhead: {} KB", node48_vec / 1024);
}
