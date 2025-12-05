use memkv::{ArenaNode, UltraNode};
use memkv::art_arena::{NodeRef, DataRef};

fn main() {
    println!("=== Detailed Size Analysis ===\n");
    
    println!("References:");
    println!("  NodeRef: {} bytes", std::mem::size_of::<NodeRef>());
    println!("  DataRef: {} bytes", std::mem::size_of::<DataRef>());
    println!("  Box<[u8; 256]>: {} bytes", std::mem::size_of::<Box<[u8; 256]>>());
    println!("  Box<[NodeRef; 16]>: {} bytes", std::mem::size_of::<Box<[NodeRef; 16]>>());
    println!("  Box<[NodeRef; 48]>: {} bytes", std::mem::size_of::<Box<[NodeRef; 48]>>());
    println!("  Option<(DataRef, u64)>: {} bytes", std::mem::size_of::<Option<(DataRef, u64)>>());
    
    println!("\nArenaNode<u64>:");
    println!("  Total size: {} bytes", std::mem::size_of::<ArenaNode<u64>>());
    
    // Calculate what each variant would be
    // Leaf: DataRef(6) + u64(8) = 14 + discriminant + padding
    // Node4: DataRef(6) + u8(1) + [u8;4] + [NodeRef;4](16) + Option<(DataRef,u64)>(24) = 51
    // Node16: DataRef(6) + u8(1) + [u8;16] + Box(8) + Option(24) = 55
    // Node48: DataRef(6) + u8(1) + Box(8) + Box(8) + Option(24) = 47
    // Node256: DataRef(6) + u16(2) + Box(8) + Option(24) = 40
    
    println!("\nUltraNode<u64>:");
    println!("  Total size: {} bytes", std::mem::size_of::<UltraNode<u64>>());
    
    println!("\n=== Memory Breakdown for 1.46M nodes ===");
    let node_count = 1_458_050usize;
    
    let arena_base = node_count * std::mem::size_of::<ArenaNode<u64>>();
    let ultra_base = node_count * std::mem::size_of::<UltraNode<u64>>();
    
    println!("ArenaNode Vec storage: {} MB", arena_base / (1024*1024));
    println!("UltraNode (if no allocation overhead): {} MB", ultra_base / (1024*1024));
    
    // With allocation overhead
    let alloc_overhead = 48; // jemalloc overhead per allocation
    let ultra_with_overhead = node_count * (std::mem::size_of::<UltraNode<u64>>() + alloc_overhead);
    println!("UltraNode with ~48 byte alloc overhead each: {} MB", ultra_with_overhead / (1024*1024));
    
    // Box allocations in ArenaArt for Node16/48/256
    // Node16 count: 49465 → each has Box<[NodeRef; 16]> = 64 bytes + ~48 overhead
    // Node48 count: 2713 → each has 2 Boxes: child_index(256b) + children(192b) + 2*48 overhead
    // Node256 count: 17 → each has Box<[NodeRef; 256]> = 1024 bytes + ~48 overhead
    let n16 = 49465;
    let n48 = 2713;
    let n256 = 17;
    
    let box_overhead = n16 * (64 + 48) + n48 * (256 + 192 + 96) + n256 * (1024 + 48);
    println!("\nArenaArt Box allocations for Node16/48/256:");
    println!("  Node16 ({}): {} MB", n16, (n16 * (64 + 48)) / (1024*1024));
    println!("  Node48 ({}): {} MB", n48, (n48 * (256 + 192 + 96)) / (1024*1024));
    println!("  Node256 ({}): {} KB", n256, (n256 * (1024 + 48)) / 1024);
    println!("  Total Box overhead: {} MB", box_overhead / (1024*1024));
}
