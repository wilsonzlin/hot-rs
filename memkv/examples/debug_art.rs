use std::io::{BufRead, BufReader};
use std::fs::File;

fn main() {
    let file = File::open("/tmp/urls_sample.txt").unwrap();
    let reader = BufReader::new(file);
    let keys: Vec<String> = reader.lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.is_empty() && l != "url")
        .take(10000)
        .collect();
    
    // Test HotArt
    println!("Testing HotArt...");
    {
        use memkv::HotArt;
        let mut tree = HotArt::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let mut failures = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) != Some(i as u64) { failures += 1; }
        }
        println!("  HotArt: {}/{} failures", failures, keys.len());
    }
    
    // Test TrueHot
    println!("Testing TrueHot...");
    {
        use memkv::TrueHot;
        let mut tree = TrueHot::new();
        for (i, key) in keys.iter().enumerate() {
            tree.insert(key.as_bytes(), i as u64);
        }
        let mut failures = 0;
        for (i, key) in keys.iter().enumerate() {
            if tree.get(key.as_bytes()) != Some(i as u64) { failures += 1; }
        }
        println!("  TrueHot: {}/{} failures", failures, keys.len());
    }
}
