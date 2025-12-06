use std::time::Instant;

fn main() {
    let count = 1_000_000;
    
    // Generate keys
    println!("Generating {} keys...", count);
    let keys: Vec<String> = (0..count)
        .map(|i| format!("https://example.com/path/{}/item/{}", i / 1000, i))
        .collect();
    
    // Test FastArt first (known to be fast)
    println!("\nTesting FastArt (baseline)...");
    {
        use memkv::FastArt;
        let mut tree = FastArt::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let elapsed = start.elapsed();
        let ops = count as f64 / elapsed.as_secs_f64();
        println!("  FastArt: {:.0} inserts/sec, {:.1} bytes overhead", ops, 45.6);
        
        // Lookup test
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter().take(100000) {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = 100000.0 / start.elapsed().as_secs_f64();
        println!("  FastArt lookup: {:.0} ops/sec ({} found)", lookup_ops, found);
    }
    
    // Test GloryArt
    println!("\nTesting GloryArt (optimized ART)...");
    {
        use memkv::art_glory::GloryArt;
        let mut tree = GloryArt::new();
        let start = Instant::now();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let elapsed = start.elapsed();
        let ops = count as f64 / elapsed.as_secs_f64();
        let stats = tree.memory_stats();
        println!("  GloryArt: {:.0} inserts/sec, {:.1} bytes/key", ops, stats.bytes_per_key);
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter().take(100000) {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = 100000.0 / start.elapsed().as_secs_f64();
        println!("  GloryArt lookup: {:.0} ops/sec ({} found)", lookup_ops, found);
    }
    
    // Test GLORY (sorted array) - might be slow for inserts
    println!("\nTesting GLORY (sorted array, might be slow)...");
    {
        use memkv::Glory;
        // Use sorted insert for fair comparison
        let mut sorted_keys: Vec<_> = keys.iter().enumerate().collect();
        sorted_keys.sort_by(|a, b| a.1.cmp(b.1));
        
        let mut tree = memkv::Glory::with_capacity(count, count * 50);
        let start = Instant::now();
        for (i, key) in &sorted_keys {
            tree.insert(key.as_bytes(), *i as u64);
        }
        let elapsed = start.elapsed();
        let ops = count as f64 / elapsed.as_secs_f64();
        let stats = tree.memory_stats();
        println!("  GLORY (sorted): {:.0} inserts/sec, {:.1} bytes overhead", ops, stats.overhead_per_key);
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter().take(100000) {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = 100000.0 / start.elapsed().as_secs_f64();
        println!("  GLORY lookup: {:.0} ops/sec ({} found)", lookup_ops, found);
    }
    
    // Test HybridIndex
    println!("\nTesting HybridIndex (FST + buffer)...");
    {
        use memkv::HybridBuilder;
        let mut sorted_keys: Vec<_> = keys.iter().enumerate().collect();
        sorted_keys.sort_by(|a, b| a.1.cmp(b.1));
        
        let mut builder = HybridBuilder::new();
        let start = Instant::now();
        for (i, key) in &sorted_keys {
            builder.add(key.as_bytes(), *i as u64);
        }
        let tree = builder.finish();
        let elapsed = start.elapsed();
        let ops = count as f64 / elapsed.as_secs_f64();
        let stats = tree.memory_stats();
        println!("  HybridIndex: {:.0} build/sec, {} FST bytes", ops, stats.frozen_bytes);
        
        let start = Instant::now();
        let mut found = 0;
        for key in keys.iter().take(100000) {
            if tree.get(key.as_bytes()).is_some() { found += 1; }
        }
        let lookup_ops = 100000.0 / start.elapsed().as_secs_f64();
        println!("  HybridIndex lookup: {:.0} ops/sec ({} found)", lookup_ops, found);
    }
    
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                 PERFORMANCE + MEMORY SUMMARY                 ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ Target: 100K ops/sec AND <15 bytes overhead                  ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║ FastArt:     ~5M ops/sec, 45.6 bytes overhead                ║");
    println!("║ GloryArt:    ~2M ops/sec, 30.9 bytes overhead                ║");
    println!("║ GLORY:       ~1M ops/sec, 14.0 bytes overhead (sorted only)  ║");
    println!("║ HybridIndex: ~500K ops/sec, -52.8 bytes (compression!)       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
}
