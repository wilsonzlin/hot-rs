fn main() {
    use memkv::ArenaNode;
    use memkv::UltraNode;
    
    println!("=== Node Size Comparison ===\n");
    
    println!("ArenaNode<u64>: {} bytes", std::mem::size_of::<ArenaNode<u64>>());
    println!("UltraNode<u64>: {} bytes", std::mem::size_of::<UltraNode<u64>>());
    
    // Calculate overhead per variant
    println!("\nWith 1.46M nodes:");
    let arena_total = 1_458_050 * std::mem::size_of::<ArenaNode<u64>>();
    let ultra_total = 1_458_050 * std::mem::size_of::<UltraNode<u64>>();
    println!("  ArenaNode total: {} MB", arena_total / (1024 * 1024));
    println!("  UltraNode total: {} MB", ultra_total / (1024 * 1024));
}
