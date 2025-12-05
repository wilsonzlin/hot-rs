//! Debug scale issue

use memkv::UltraCompactArt;

fn main() {
    let content = std::fs::read_to_string("urls_sample.txt").unwrap();
    let urls: Vec<&str> = content.lines().collect();
    
    println!("Total URLs in file: {}", urls.len());
    
    // Test at different scales
    for size in [1000, 5000, 10000, 50000, 100000, 500000, urls.len()] {
        if size > urls.len() { continue; }
        
        let mut tree: UltraCompactArt<u64> = UltraCompactArt::new();
        for (i, url) in urls[..size].iter().enumerate() {
            tree.insert(url.as_bytes(), i as u64);
        }
        
        let mut ok_count = 0;
        for (i, url) in urls[..size].iter().enumerate() {
            if tree.get(url.as_bytes()) == Some(&(i as u64)) {
                ok_count += 1;
            }
        }
        
        let pct = 100.0 * ok_count as f64 / size as f64;
        let status = if ok_count == size { "OK" } else { "FAIL" };
        println!("  {} URLs: {}/{} ({:.1}%) {}", size, ok_count, size, pct, status);
    }
}
