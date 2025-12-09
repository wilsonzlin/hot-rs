//! Scale test - load a URL file into memory and measure memory/throughput.
//!
//! Default path: data/urls_500k.txt
//! For the full crawl, pass /state/static.wilsonl.in/urls.txt (one URL per line).

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Force jemalloc to return unused memory to the OS and get accurate allocated bytes.
fn get_allocated() -> usize {
    // Advance epoch to get fresh statistics
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    // Purge unused dirty pages to get accurate measurement
    unsafe {
        let _ = tikv_jemalloc_ctl::raw::write(b"arena.0.purge\0", 0u64);
    }
    tikv_jemalloc_ctl::epoch::advance().unwrap();
    tikv_jemalloc_ctl::stats::allocated::read().unwrap()
}

use clap::{Parser, ValueEnum};
use memkv::{FastArt, InlineHot, HOT};
use rand::seq::SliceRandom;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::panic;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Structure {
    BTreeMap,
    InlineHot,
    Hot,
    FastArt,
}

impl Structure {
    fn name(&self) -> &'static str {
        match self {
            Structure::BTreeMap => "BTreeMap",
            Structure::InlineHot => "InlineHot",
            Structure::Hot => "HOT",
            Structure::FastArt => "FastArt",
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "scale_test")]
#[command(about = "Stream a URL file and measure memory/throughput for various data structures")]
struct Args {
    /// Path to the input file (one URL per line)
    #[arg(short, long, default_value = "data/urls_500k.txt")]
    path: String,

    /// Skip verification pass (lookup all keys after insertion)
    #[arg(long, default_value_t = false)]
    no_verify: bool,

    /// Data structures to test (comma-separated or multiple flags)
    #[arg(short, long, value_enum, value_delimiter = ',', default_values_t = vec![
        Structure::BTreeMap,
        Structure::InlineHot,
        Structure::Hot,
        Structure::FastArt,
    ])]
    structures: Vec<Structure>,
}

struct VerifyResult {
    found: usize,
    total: usize,
    elapsed: Duration,
}

struct Stats {
    count: usize,
    raw_bytes: usize,
    total_bytes: usize,
    insert_time: Duration,
    verify: Option<VerifyResult>,
}

/// Load all URLs from a file into memory using parallel parsing.
fn load_urls(path: &str) -> io::Result<Vec<Vec<u8>>> {
    let data = fs::read_to_string(path)?;
    let urls: Vec<Vec<u8>> = data
        .par_lines()
        .map(|line| line.trim().as_bytes().to_vec())
        .collect();
    Ok(urls)
}

fn print_table_header(path: &str, count: usize, raw_bytes: usize, verify: bool) {
    println!("Input: {path}");
    println!("Loaded {count} URLs ({:.1} MB raw)", raw_bytes as f64 / 1e6);
    println!();
    println!(
        "{:<12} {:>12} {:>10} {:>10} {:>12} {:>10} {:>6}",
        "Structure",
        "Total MB",
        "Overhead",
        "Index",
        "Insert/s",
        "Lookup/s",
        if verify { "OK?" } else { "Check" }
    );
    println!(
        "{:<12} {:>12} {:>10} {:>10} {:>12} {:>10} {:>6}",
        "", "", "B/K", "B/K", "", "", ""
    );
    println!("{}", "─".repeat(74));
}

fn print_stats(name: &str, stats: &Stats) {
    let total_mb = stats.total_bytes as f64 / 1e6;
    let (overhead, index) = if stats.count > 0 {
        let overhead =
            (stats.total_bytes as f64 - stats.raw_bytes as f64) / stats.count as f64;
        (overhead, overhead - 8.0)
    } else {
        (0.0, 0.0)
    };
    let insert_rate = if stats.insert_time.as_secs_f64() > 0.0 {
        stats.count as f64 / stats.insert_time.as_secs_f64()
    } else {
        0.0
    };

    let (lookup_rate, ok) = match &stats.verify {
        Some(v) if v.elapsed.as_secs_f64() > 0.0 => (
            v.total as f64 / v.elapsed.as_secs_f64(),
            if v.found == v.total { "✓" } else { "✗" },
        ),
        Some(_) => (0.0, "✗"),
        None => (0.0, "-"),
    };

    println!(
        "{:<12} {:>12.1} {:>10.1} {:>10.1} {:>12.0} {:>10.0} {:>6}",
        name, total_mb, overhead, index, insert_rate, lookup_rate, ok
    );
}

fn print_skipped(name: &str, reason: &str) {
    println!(
        "{:<12} {:>12} {:>10} {:>10} {:>12} {:>10} {:>6}",
        name, "-", "-", "-", "-", "-", reason
    );
}

fn run_btree(urls: &[Vec<u8>], verify: bool) -> Stats {
    let raw_bytes: usize = urls.iter().map(|u| u.len()).sum();

    // Measure memory before
    let before = get_allocated();

    let mut map: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    let start = Instant::now();
    for (i, url) in urls.iter().enumerate() {
        map.insert(url.clone(), i as u64);
    }
    let insert_time = start.elapsed();

    // Measure memory after
    let after = get_allocated();

    let verify = if verify {
        let start = Instant::now();
        let mut found = 0usize;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url).copied() == Some(i as u64) {
                found += 1;
            }
        }
        Some(VerifyResult {
            found,
            total: urls.len(),
            elapsed: start.elapsed(),
        })
    } else {
        None
    };

    Stats {
        count: urls.len(),
        raw_bytes,
        total_bytes: after.saturating_sub(before),
        insert_time,
        verify,
    }
}

fn run_inline_hot(urls: &[Vec<u8>], verify: bool) -> Stats {
    let raw_bytes: usize = urls.iter().map(|u| u.len()).sum();

    let before = get_allocated();

    let mut map = InlineHot::new();
    let start = Instant::now();
    for (i, url) in urls.iter().enumerate() {
        map.insert(url, i as u64);
    }
    let insert_time = start.elapsed();
    map.shrink_to_fit();

    let after = get_allocated();

    let verify = if verify {
        let start = Instant::now();
        let mut found = 0usize;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url) == Some(i as u64) {
                found += 1;
            }
        }
        Some(VerifyResult {
            found,
            total: urls.len(),
            elapsed: start.elapsed(),
        })
    } else {
        None
    };

    Stats {
        count: urls.len(),
        raw_bytes,
        total_bytes: after.saturating_sub(before),
        insert_time,
        verify,
    }
}

fn run_hot(urls: &[Vec<u8>], verify: bool) -> Stats {
    let raw_bytes: usize = urls.iter().map(|u| u.len()).sum();

    let before = get_allocated();

    let mut map = HOT::new();
    let start = Instant::now();
    for (i, url) in urls.iter().enumerate() {
        map.insert(url, i as u64);
    }
    let insert_time = start.elapsed();
    map.shrink_to_fit();

    let after = get_allocated();

    let verify = if verify {
        let start = Instant::now();
        let mut found = 0usize;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url) == Some(i as u64) {
                found += 1;
            }
        }
        Some(VerifyResult {
            found,
            total: urls.len(),
            elapsed: start.elapsed(),
        })
    } else {
        None
    };

    Stats {
        count: urls.len(),
        raw_bytes,
        total_bytes: after.saturating_sub(before),
        insert_time,
        verify,
    }
}

fn run_fast_art(urls: &[Vec<u8>], verify: bool) -> Stats {
    let raw_bytes: usize = urls.iter().map(|u| u.len()).sum();

    let before = get_allocated();

    let mut map = FastArt::new();
    let start = Instant::now();
    for (i, url) in urls.iter().enumerate() {
        map.insert(url, i as u64);
    }
    let insert_time = start.elapsed();

    let after = get_allocated();

    let verify = if verify {
        let start = Instant::now();
        let mut found = 0usize;
        for (i, url) in urls.iter().enumerate() {
            if map.get(url) == Some(i as u64) {
                found += 1;
            }
        }
        Some(VerifyResult {
            found,
            total: urls.len(),
            elapsed: start.elapsed(),
        })
    } else {
        None
    };

    Stats {
        count: urls.len(),
        raw_bytes,
        total_bytes: after.saturating_sub(before),
        insert_time,
        verify,
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let verify = !args.no_verify;

    println!("Scale test");
    println!("Loading URLs from {}...", args.path);

    // Load all URLs into memory first (not counted in benchmark)
    let mut urls = load_urls(&args.path)?;
    let raw_bytes: usize = urls.iter().map(|u| u.len()).sum();

    println!("Shuffling...");
    urls.shuffle(&mut rand::thread_rng());

    // Force GC before benchmarks to establish clean baseline
    let _ = get_allocated();

    print_table_header(&args.path, urls.len(), raw_bytes, verify);

    for structure in &args.structures {
        let name = structure.name();
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            match structure {
                Structure::BTreeMap => run_btree(&urls, verify),
                Structure::InlineHot => run_inline_hot(&urls, verify),
                Structure::Hot => run_hot(&urls, verify),
                Structure::FastArt => run_fast_art(&urls, verify),
            }
        }));
        match result {
            Ok(stats) => print_stats(name, &stats),
            Err(e) => {
                let reason = if let Some(s) = e.downcast_ref::<&str>() {
                    if s.contains("addressable") { "u32" } else { "panic" }
                } else if let Some(s) = e.downcast_ref::<String>() {
                    if s.contains("addressable") { "u32" } else { "panic" }
                } else {
                    "panic"
                };
                print_skipped(name, reason);
            }
        }
    }

    println!("{}", "─".repeat(74));
    println!("Overhead = (total - raw_keys) / count; Index = overhead - 8 (u64 value)");
    Ok(())
}
