//! Baseline benchmarks comparing MemKV to standard library collections.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use std::collections::{BTreeMap, HashMap};
use memkv::MemKV;

fn generate_keys(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("user:{:08}", i)).collect()
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert");
    
    for size in [1_000, 10_000, 100_000].iter() {
        let keys = generate_keys(*size);
        
        group.bench_with_input(BenchmarkId::new("BTreeMap", size), size, |b, _| {
            b.iter(|| {
                let mut map: BTreeMap<String, u64> = BTreeMap::new();
                for (i, key) in keys.iter().enumerate() {
                    map.insert(key.clone(), i as u64);
                }
                black_box(map)
            });
        });
        
        group.bench_with_input(BenchmarkId::new("HashMap", size), size, |b, _| {
            b.iter(|| {
                let mut map: HashMap<String, u64> = HashMap::new();
                for (i, key) in keys.iter().enumerate() {
                    map.insert(key.clone(), i as u64);
                }
                black_box(map)
            });
        });
        
        group.bench_with_input(BenchmarkId::new("MemKV", size), size, |b, _| {
            b.iter(|| {
                let kv: MemKV<u64> = MemKV::new();
                for (i, key) in keys.iter().enumerate() {
                    kv.insert(key.as_bytes(), i as u64);
                }
                black_box(kv)
            });
        });
    }
    
    group.finish();
}

fn bench_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("lookup");
    
    for size in [1_000, 10_000, 100_000].iter() {
        let keys = generate_keys(*size);
        
        // Prepare BTreeMap
        let mut btree: BTreeMap<String, u64> = BTreeMap::new();
        for (i, key) in keys.iter().enumerate() {
            btree.insert(key.clone(), i as u64);
        }
        
        // Prepare HashMap
        let mut hashmap: HashMap<String, u64> = HashMap::new();
        for (i, key) in keys.iter().enumerate() {
            hashmap.insert(key.clone(), i as u64);
        }
        
        // Prepare MemKV
        let memkv: MemKV<u64> = MemKV::new();
        for (i, key) in keys.iter().enumerate() {
            memkv.insert(key.as_bytes(), i as u64);
        }
        
        group.bench_with_input(BenchmarkId::new("BTreeMap", size), size, |b, _| {
            b.iter(|| {
                let mut sum = 0u64;
                for key in keys.iter() {
                    if let Some(v) = btree.get(key) {
                        sum += v;
                    }
                }
                black_box(sum)
            });
        });
        
        group.bench_with_input(BenchmarkId::new("HashMap", size), size, |b, _| {
            b.iter(|| {
                let mut sum = 0u64;
                for key in keys.iter() {
                    if let Some(v) = hashmap.get(key) {
                        sum += v;
                    }
                }
                black_box(sum)
            });
        });
        
        group.bench_with_input(BenchmarkId::new("MemKV", size), size, |b, _| {
            b.iter(|| {
                let mut sum = 0u64;
                for key in keys.iter() {
                    if let Some(v) = memkv.get(key.as_bytes()) {
                        sum += v;
                    }
                }
                black_box(sum)
            });
        });
    }
    
    group.finish();
}

criterion_group!(benches, bench_insert, bench_lookup);
criterion_main!(benches);
