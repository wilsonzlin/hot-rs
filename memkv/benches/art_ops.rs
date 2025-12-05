//! Benchmarks for ART operations.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use memkv::MemKV;

fn generate_random_keys(n: usize, seed: u64) -> Vec<Vec<u8>> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    
    (0..n).map(|i| {
        let mut hasher = DefaultHasher::new();
        (seed + i as u64).hash(&mut hasher);
        let hash = hasher.finish();
        format!("key:{:016x}", hash).into_bytes()
    }).collect()
}

fn generate_sequential_keys(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| format!("key:{:08}", i).into_bytes()).collect()
}

fn generate_url_like_keys(n: usize) -> Vec<Vec<u8>> {
    let domains = ["example.com", "test.org", "demo.net", "sample.io"];
    let paths = ["users", "posts", "comments", "api/v1", "api/v2"];
    
    (0..n).map(|i| {
        let domain = domains[i % domains.len()];
        let path = paths[(i / domains.len()) % paths.len()];
        let id = i / (domains.len() * paths.len());
        format!("{}/{}/{}", domain, path, id).into_bytes()
    }).collect()
}

fn bench_insert_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_patterns");
    let size = 10_000;
    
    let sequential = generate_sequential_keys(size);
    let random = generate_random_keys(size, 42);
    let url_like = generate_url_like_keys(size);
    
    group.bench_function("sequential", |b| {
        b.iter(|| {
            let kv: MemKV<u64> = MemKV::new();
            for (i, key) in sequential.iter().enumerate() {
                kv.insert(key.as_slice(), i as u64);
            }
            black_box(kv)
        });
    });
    
    group.bench_function("random", |b| {
        b.iter(|| {
            let kv: MemKV<u64> = MemKV::new();
            for (i, key) in random.iter().enumerate() {
                kv.insert(key.as_slice(), i as u64);
            }
            black_box(kv)
        });
    });
    
    group.bench_function("url_like", |b| {
        b.iter(|| {
            let kv: MemKV<u64> = MemKV::new();
            for (i, key) in url_like.iter().enumerate() {
                kv.insert(key.as_slice(), i as u64);
            }
            black_box(kv)
        });
    });
    
    group.finish();
}

fn bench_prefix_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefix_scan");
    
    // Create a store with URL-like keys
    let kv: MemKV<u64> = MemKV::new();
    let size = 100_000;
    
    for i in 0..size {
        let key = format!("domain{}.com/path{}/item{}", i % 100, (i / 100) % 100, i);
        kv.insert(key.as_bytes(), i as u64);
    }
    
    group.bench_function("prefix_1%", |b| {
        b.iter(|| {
            let count = kv.prefix(b"domain0.com/").len();
            black_box(count)
        });
    });
    
    group.bench_function("prefix_0.01%", |b| {
        b.iter(|| {
            let count = kv.prefix(b"domain0.com/path0/").len();
            black_box(count)
        });
    });
    
    group.finish();
}

fn bench_range_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("range_query");
    
    let kv: MemKV<u64> = MemKV::new();
    let size = 100_000;
    
    for i in 0..size {
        let key = format!("key:{:08}", i);
        kv.insert(key.as_bytes(), i as u64);
    }
    
    group.bench_function("range_1%", |b| {
        b.iter(|| {
            let count = kv.range(b"key:00010000", b"key:00011000").len();
            black_box(count)
        });
    });
    
    group.bench_function("range_10%", |b| {
        b.iter(|| {
            let count = kv.range(b"key:00010000", b"key:00020000").len();
            black_box(count)
        });
    });
    
    group.finish();
}

criterion_group!(benches, bench_insert_patterns, bench_prefix_scan, bench_range_query);
criterion_main!(benches);
