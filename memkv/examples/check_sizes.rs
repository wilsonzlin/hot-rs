fn main() {
    use memkv::art_optimized::*;
    use memkv::art_arena::*;
    
    println!("CompactNode<u64>: {} bytes", std::mem::size_of::<memkv::art_optimized::CompactNode<u64>>());
    println!("ArenaNode<u64>: {} bytes", std::mem::size_of::<ArenaNode<u64>>());
    
    // Check what's making CompactNode so big
    println!("\nCompactNode variant sizes:");
    println!("  Leaf data: {} bytes", std::mem::size_of::<(u32, u16, u64)>());
    println!("  Node4 inline prefix [u8;12]: {} bytes", 12);
    println!("  Node16 keys [u8;16]: {} bytes", 16);
    println!("  Node16 children [NodeIdx;16]: {} bytes", 16 * 4);
    println!("  Node48 child_idx Box<[u8;256]>: {} bytes", 8);
    println!("  Node48 children [NodeIdx;48]: {} bytes", 48 * 4);
    println!("  Node256 children Box<[NodeIdx;256]>: {} bytes", 8);
}
