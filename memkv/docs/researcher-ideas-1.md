DuckDB uses their own implementation of Adaptive Radix Tree (ART) indexes, built directly into DuckDB's codebase rather than using an external library. You can find the source code at `src/execution/index/art/` in the DuckDB repository.

Their implementation is based on the original ART paper from the HyPer database team at TU Munich: "The Adaptive Radix Tree: ARTful Indexing for Main-Memory Databases" by Viktor Leis et al. ART was designed by the HyPer team, and HyPer later got acquired by Tableau.

DuckDB chose ART because it provides:
- A compact structure where internal nodes can fit in CPU caches, making it more cache-conscious than B+-Trees
- Fast point queries with O(k) worst case, sufficient for constraint checking
- No dramatic regression on insertions, unlike hash tables that must be rebuilt at certain sizes

DuckDB has extended the original ART design with custom features like swizzlable pointers for persistence, using the most significant bit as a flag to indicate whether the value is a memory address or block storage information.




Here's a comprehensive overview of the state of the art for Adaptive Radix Trees:

## Foundational Research

**Original ART (2013)** - Viktor Leis, Alfons Kemper, and Thomas Neumann at TU Munich introduced ART in their ICDE 2013 paper. The core innovations were:
- Four adaptive node sizes (4, 16, 48, 256 children) to balance space and performance
- Path compression and lazy expansion to reduce tree height
- Guaranteed overhead of no more than 52 bytes per key, though in practice much lower

## Concurrency Research

**Optimistic Lock Coupling & ROWEX (2016)** - The same TU Munich team published synchronization protocols for ART at DaMoN 2016:
- Optimistic Lock Coupling and Read-Optimized Write EXclusion (ROWEX) perform and scale very well while being much easier to implement than lock-free techniques
- Uses epoch-based memory reclamation to defer freeing nodes that readers might still be accessing

## Alternative Designs

**Height Optimized Trie (HOT)** - Addresses ART's weakness with string keys where the average fanout is much lower at lower levels of the tree due to sparse key distributions:
- Dynamically varies the number of bits considered at each node, enabling a consistently high fanout
- The height of HOT structures is provably minimal and the structure is deterministically defined regardless of insertion order
- Implementation available at github.com/speedskater/hot with SIMD optimizations requiring AVX-2 and BMI-2

**START (2020)** - Self-Tuning Adaptive Radix Tree uses nodes spanning multiple keybytes, performing on average 85% faster than regular ART on read-only workloads and 45% faster for read-mostly workloads

## Persistent Memory Variants

**WORT/WOART (FAST 2017)** - Write Optimal Radix Tree variants only use one 8-byte failure-atomic write per update to guarantee consistency without requiring duplicate copies for logging or CoW

**RECIPE (SOSP 2019)** - P-ART converts ART for persistent memory using ROWEX for concurrency, suitable for insertion-dominated workloads with few range queries

**PFtree (DASFAA 2023)** - Optimized for eADR platforms, reduces PM line access by storing key-value pairs in leaf arrays directly and reduces memory allocations in critical paths

## Disaggregated Memory

**SMART (OSDI 2023)** - First radix tree for disaggregated memory achieving 6.1x higher throughput under write-intensive workloads and 2.8x under read-only workloads compared to state-of-the-art B+ trees on DM:
- Uses hybrid concurrency control with lock-free internal nodes and fine-grained lock-based leaf nodes
- Source code available at github.com/dmemsys/SMART

## Notable Implementations

| Implementation | Language | Features |
|----------------|----------|----------|
| **DuckDB** | C++ | Custom implementation with swizzlable pointers for disk persistence |
| **libart** | C | C99 implementation by Armon Dadgar, simple single-threaded |
| **ARTSynchronized** | C++ | Implements both OLC and ROWEX synchronization |
| **Congee** | Rust | ART-OLC with proper SIMD support, optimized for fixed 8-byte keys |
| **art-rs** | Rust | Includes experimental prefix-caching using hash table mapping from key prefixes to interior nodes |

## Current Research Directions

- **Learned indexes** - Comparing/hybridizing with ML-based approaches
- **CXL heterogeneous memory** - Node-grained data placement schemes
- **Index cracking** - Incrementally building ART indices during query execution, reducing average initialization time by 75%





Here's a comprehensive overview of the state of the art for maximizing memory efficiency in Adaptive Radix Trees:

## Core ART Memory Techniques

**Horizontal Compression (Adaptive Node Sizes)**
ART uses adaptive nodes that solve the sparsity problem by locally optimizing space consumption at each node. When keys are sparse, many pointers of inner nodes are null, resulting in wasted space. Adaptive nodes ensure any key distribution is stored compactly.

The standard node types:
- **Node4**: 4 keys + 4 pointers (smallest)
- **Node16**: Uses 128-bit SIMD instructions (SSE2 and NEON) to map a key onto an edge
- **Node48**: Uses a 256-element key index array but only 48 element child pointer array, allowing O(1) key search while saving memory by having space for only 48 pointers
- **Node256**: Direct indexing

**Vertical Compression**
- Path compression and lazy expansion allow ART to efficiently index long keys by collapsing nodes and decreasing height
- Inner nodes with only one child are merged with their parent, and each node reserves a fixed number of bytes to store the prefix
- Pointer tagging is used to tell inner nodes apart from leaf nodes

**Worst-Case Bounds**
ART can guarantee that the overhead is no more than 52 bytes per key, though in practice it is much lower.

## Domain-Specific Optimizations

**Custom Node Sizes** - For DNS hostnames, researchers introduced:
A node of size 32 which benefits from 256-bit SIMD instructions (AVX2), and a node of size 38 that stores hostname-only entries using an alternate key scheme

**Key Space Compression** - By transforming uppercase letters to lowercase, 26 values are freed and the range can be compressed to lower worst-case space consumption for nodes of size 48 and 256

Reducing the prefix vector from 10 to 9 bytes saves 32-bits for each inner node

## Alternative Space-Efficient Trie Designs

**Height Optimized Trie (HOT)**
HOT dynamically varies the number of bits considered at each node, enabling a consistently high fanout and avoiding the sparsity problem that plagues other trie variants. Space consumption is reduced and tree height is minimized.

**HAT-trie** - A cache-conscious trie combining a trie with cache-conscious hash tables. It is the most efficient trie-based data structure for managing variable-length strings in-memory while maintaining sort order.

The key insight: The burst-trie reduces space by collapsing trie-chains into buckets. HAT-trie uses an array hash table as container in its leaf nodes.

**Hyperion**
A trie-based main-memory key-value store achieving extreme space efficiency. In contrast to other data structures, Hyperion does not depend on CPU vector units, but scans the data structure linearly. Combined with a custom memory allocator, Hyperion accomplishes remarkable data density while its performance-to-memory ratio is more than two times better than the best alternatives.

## Entropy-Coded Tries

**SILT SortedStore** - Uses an entropy-coded trie providing very space efficient indexing: the average is 0.4 bytes per key (for 20-byte keys)

## Memory Layout Optimizations

| Technique | Description |
|-----------|-------------|
| **Pointer tagging** | Store values directly in pointer slots, distinguishing them based on the highest bit - 0 for child node pointer, 1 for value pointer |
| **Implicit keys** | Keys are implicitly stored in the tree structure and can be reconstructed from paths to leaf nodes, saving space because keys don't need explicit storage |
| **Custom allocators** | Hyperion uses custom memory allocators to pack nodes efficiently |
| **Fixed-size prefixes** | Fixed prefix sizes avoid memory fragmentation; if more space is required, lookups skip remaining bytes and compare at the leaf |

## Comparison of Space Efficiency

| Structure | Space per Key | Notes |
|-----------|--------------|-------|
| Original ART | ~8.1 bytes avg, 52 bytes worst | For integers |
| HOT | Lower than ART for strings | Varies by fanout parameter |
| HAT-trie | Close to hash tables | Best for strings with sort order |
| Hyperion | Extreme efficiency | Custom allocator required |
| SILT trie | 0.4 bytes/key | Entropy-coded, read-only |

## Key Trade-offs

1. **SIMD vs. Memory** - Implementations like Judy, ART, or HOT optimize internal alignments for cache and vector unit efficiency, but these measures can have a negative impact on memory efficiency

2. **Node granularity** - 64-bit integer keys are too short to notice the positive effect of path compression - both ART versions have essentially the same performance, but the version without path compression can be more space-efficient

3. **Sparse vs. dense data** - For sparse distributions, ART can seem wasteful with memory consumption compared to hash tables; for dense distributions, ART's adaptivity provides significant advantages

The current frontier focuses on learned indexes (like LITS) that combine ML models with trie structures, and specialized designs for new memory technologies (CXL, persistent memory) that have different performance characteristics.
