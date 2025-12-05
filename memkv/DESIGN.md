
---

## 11. Final Implementation Results

### Accurate Memory Measurements with jemalloc

**Critical Finding**: Previous RSS-based measurements were misleading. Using jemalloc's allocation tracking API (`tikv-jemalloc-ctl`) provides accurate results.

### Results (967K URL Dataset, 46 MB raw key data)

| Implementation | Memory | Bytes/Key | vs BTreeMap |
|---------------|--------|-----------|-------------|
| **FrozenLayer (FST)** | **40 MB** | **44** | **-65%** |
| std::BTreeMap | 115 MB | 125 | baseline |
| ArenaArt | 180 MB | 195 | +57% |
| UltraCompactArt | 192 MB | 208 | +67% |

### Key Findings

1. **FST is the clear winner for immutable data**
   - 23 MB pure FST size (25.5 bytes/key)
   - 2x compression vs raw key data
   - 65% less memory than BTreeMap

2. **ART is not competitive for moderate-size datasets**
   - Too many nodes (1.46M for 967K keys)
   - Per-allocation overhead dominates (48 bytes/node in jemalloc)

3. **BTreeMap is highly optimized**
   - High fanout (16-32 keys/node) → fewer nodes
   - Standard library implementation is excellent

### Per-Allocation Overhead Analysis

jemalloc adds ~48 bytes overhead per allocation:
- UltraCompactArt: 1.46M Box allocations × 48 bytes = 70 MB overhead
- ArenaArt eliminates this by using Vec-based storage

Node size comparison:
- UltraNode: 72 bytes + 48 bytes overhead = 120 bytes/node
- ArenaNode: 56 bytes (in Vec, no overhead)

### Recommendations

1. **For read-only/frozen data**: Use FrozenLayer (FST)
2. **For mutable data**: Use BTreeMap (surprisingly efficient)
3. **For hybrid workloads**: FST base + mutable delta

### Implementation Notes

- Use `tikv-jemalloc-ctl` for accurate memory measurement
- RSS measurements are unreliable due to allocator memory reuse
- FST requires sorted input but provides exceptional compression
