//! Benchmarks for ART operations.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use memkv::FastArt;
use std::collections::BTreeMap;

fn generate_sequential_keys(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| format!("key:{:08}", i).into_bytes()).collect()
}

fn generate_url_like_keys(n: usize) -> Vec<Vec<u8>> {
    let domains = ["example.com", "test.org", "demo.net", "sample.io"];
    let paths = ["users", "posts", "comments", "api/v1", "api/v2"];

    (0..n)
        .map(|i| {
            let domain = domains[i % domains.len()];
            let path = paths[(i / domains.len()) % paths.len()];
            let id = i / (domains.len() * paths.len());
            format!("{}/{}/{}", domain, path, id).into_bytes()
        })
        .collect()
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert");

    for size in [1_000, 10_000, 100_000] {
        let keys = generate_sequential_keys(size);

        group.bench_with_input(BenchmarkId::new("FastArt", size), &keys, |b, keys| {
            b.iter(|| {
                let mut art = FastArt::new();
                for (i, key) in keys.iter().enumerate() {
                    art.insert(key, i as u64);
                }
                black_box(art)
            });
        });

        group.bench_with_input(BenchmarkId::new("BTreeMap", size), &keys, |b, keys| {
            b.iter(|| {
                let mut map: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
                for (i, key) in keys.iter().enumerate() {
                    map.insert(key.clone(), i as u64);
                }
                black_box(map)
            });
        });
    }

    group.finish();
}

fn bench_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("lookup");

    for size in [1_000, 10_000, 100_000] {
        let keys = generate_sequential_keys(size);

        let mut art = FastArt::new();
        for (i, key) in keys.iter().enumerate() {
            art.insert(key, i as u64);
        }

        let mut btree: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (i, key) in keys.iter().enumerate() {
            btree.insert(key.clone(), i as u64);
        }

        group.bench_with_input(BenchmarkId::new("FastArt", size), &keys, |b, keys| {
            b.iter(|| {
                let mut sum = 0u64;
                for key in keys.iter() {
                    if let Some(v) = art.get(key) {
                        sum += v;
                    }
                }
                black_box(sum)
            });
        });

        group.bench_with_input(BenchmarkId::new("BTreeMap", size), &keys, |b, keys| {
            b.iter(|| {
                let mut sum = 0u64;
                for key in keys.iter() {
                    if let Some(v) = btree.get(key) {
                        sum += *v;
                    }
                }
                black_box(sum)
            });
        });
    }

    group.finish();
}

fn bench_url_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("url_patterns");

    let keys = generate_url_like_keys(10_000);

    group.bench_function("FastArt/insert", |b| {
        b.iter(|| {
            let mut art = FastArt::new();
            for (i, key) in keys.iter().enumerate() {
                art.insert(key, i as u64);
            }
            black_box(art)
        });
    });

    let mut art = FastArt::new();
    for (i, key) in keys.iter().enumerate() {
        art.insert(key, i as u64);
    }

    group.bench_function("FastArt/lookup", |b| {
        b.iter(|| {
            let mut sum = 0u64;
            for key in keys.iter() {
                if let Some(v) = art.get(key) {
                    sum += v;
                }
            }
            black_box(sum)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_insert, bench_lookup, bench_url_patterns);
criterion_main!(benches);
