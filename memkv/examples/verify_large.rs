//! Test with large URL dataset to verify u48 pointers work

use memkv::InlineHot;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::Instant;
use rand::seq::SliceRandom;

fn main() {
    let path = std::env::args().nth(1).unwrap_or("/state/static.wilsonl.in/urls.txt".to_string());
    let limit: usize = std::env::args().nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000_000);
    
    println!("Loading up to {} URLs from {}...", limit, path);
    
    let file = File::open(&path).expect("Failed to open file");
    let reader = BufReader::new(file);
    
    let start = Instant::now();
    let mut urls: Vec<Vec<u8>> = reader.lines()
        .take(limit)
        .filter_map(|l| l.ok())
        .map(|l| l.trim().as_bytes().to_vec())
        .collect();
    
    let raw_bytes: usize = urls.iter().map(|u| u.len()).sum();
    println!("Loaded {} URLs ({:.2} GB raw) in {:.1}s", 
        urls.len(), raw_bytes as f64 / 1e9, start.elapsed().as_secs_f64());
    
    // Shuffle
    println!("Shuffling...");
    urls.shuffle(&mut rand::thread_rng());
    
    // Insert
    println!("Inserting into InlineHot...");
    let mut map = InlineHot::new();
    let start = Instant::now();
    for (i, url) in urls.iter().enumerate() {
        map.insert(url, i as u64);
        if i > 0 && i % 1_000_000 == 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let rate = i as f64 / elapsed;
            println!("  {:>10} keys, {:.1}M/s, memory: {:.2} GB", 
                i, rate / 1e6, map.memory_usage_actual() as f64 / 1e9);
        }
    }
    let insert_time = start.elapsed();
    
    map.shrink_to_fit();
    let memory = map.memory_usage_actual();
    
    println!("\nInsert complete:");
    println!("  Keys: {}", map.len());
    println!("  Time: {:.1}s ({:.0} keys/s)", insert_time.as_secs_f64(), 
        urls.len() as f64 / insert_time.as_secs_f64());
    println!("  Memory: {:.2} GB", memory as f64 / 1e9);
    println!("  Overhead: {:.1} B/key", 
        (memory as f64 - raw_bytes as f64) / urls.len() as f64);
    
    // Verify 10K random samples
    println!("\nVerifying 10K random samples...");
    let mut sample_indices: Vec<usize> = (0..urls.len()).collect();
    sample_indices.shuffle(&mut rand::thread_rng());
    let sample_indices: Vec<usize> = sample_indices.into_iter().take(10_000).collect();
    
    // Build expected values for samples
    let mut expected: std::collections::HashMap<&Vec<u8>, u64> = std::collections::HashMap::new();
    for (i, url) in urls.iter().enumerate() {
        expected.insert(url, i as u64);
    }
    
    let start = Instant::now();
    let mut correct = 0usize;
    for &i in &sample_indices {
        let url = &urls[i];
        let exp = *expected.get(url).unwrap();
        if map.get(url) == Some(exp) {
            correct += 1;
        }
    }
    let verify_time = start.elapsed();
    
    println!("  Verified {}/{} samples in {:.2}s", correct, sample_indices.len(), verify_time.as_secs_f64());
    
    if correct == sample_indices.len() {
        println!("\n✓ Verification passed!");
    } else {
        println!("\n✗ Verification failed!");
    }
}
