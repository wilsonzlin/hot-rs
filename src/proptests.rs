use super::*;

use proptest::prelude::*;
use std::collections::BTreeMap;

fn validate_tree<V>(t: &HotTree<V>) {
    if std::mem::size_of::<V>() == 0 {
        assert_eq!(
            t.zst_values.len(),
            t.count,
            "ZST values must track live key count"
        );
    }

    let mut stack: Vec<Ptr> = Vec::new();
    if !t.root.is_null() {
        stack.push(t.root);
    }

    let mut leaf_count = 0usize;
    while let Some(ptr) = stack.pop() {
        assert!(!ptr.is_null(), "NULL pointer inside tree");

        if ptr.is_leaf() {
            assert!(!ptr.is_tombstone(), "tombstone leaf should be unreachable");

            leaf_count += 1;

            if std::mem::size_of::<V>() != 0 {
                let idx = t.get_leaf_value_idx(ptr.leaf_off());
                assert!(
                    t.values[idx].is_some(),
                    "reachable leaf must have a live value"
                );
            }
            continue;
        }

        let node_off = ptr.node_off();
        let tag = t.nodes.tag(node_off);
        let n = t.node_entry_count(node_off);
        assert!(
            (2..=MAX_COMPOUND_ENTRIES).contains(&n),
            "invalid entry count: {n}"
        );
        if tag == NODE_TWO_ENTRIES {
            assert_eq!(n, 2, "NODE_TWO_ENTRIES must have n=2");
        }

        let mut max_child_h = 0u8;
        for i in 0..n {
            let child = t.node_entry_ptr(node_off, i);
            assert!(!child.is_null(), "node child pointer is NULL");
            max_child_h = max_child_h.max(t.ptr_height(child));
            stack.push(child);
        }
        assert_eq!(
            t.nodes.height(node_off),
            max_child_h.saturating_add(1),
            "stored node height must match children"
        );

        if hot_is_hot_node(tag) {
            assert_ne!(t.nodes.hot_mapping(node_off).num_bits(), 0);
            let mut prev = 0u32;
            for i in 0..n {
                let pk = t.nodes.hot_partial_key_u32_at(node_off, i);
                if i == 0 {
                    assert_eq!(pk, 0, "first sparse partial key must be 0");
                } else {
                    assert!(
                        pk >= prev,
                        "sparse partial keys must be non-decreasing (idx={i})"
                    );
                }
                prev = pk;
            }
        }
    }

    assert_eq!(
        leaf_count, t.count,
        "reachable leaf count must match HotTree::len"
    );
}

#[derive(Clone, Debug)]
enum Op<V> {
    Insert(Vec<u8>, V),
    Remove(Vec<u8>),
    Get(Vec<u8>),
    Compact,
}

fn key_strategy() -> impl Strategy<Value = Vec<u8>> + Clone {
    // URLs and most string-like keys never contain 0x00 bytes. This avoids the
    // current limitation where keys that differ only by trailing 0x00 bytes
    // are not distinguishable at the bit level.
    prop::collection::vec(1u8..=255, 0..=64)
}

fn ops_strategy_u64() -> impl Strategy<Value = Vec<Op<u64>>> {
    let key = key_strategy();
    let op = prop_oneof![
        50 => (key.clone(), any::<u64>()).prop_map(|(k, v)| Op::Insert(k, v)),
        25 => key.clone().prop_map(Op::Remove),
        24 => key.clone().prop_map(Op::Get),
        1 => Just(Op::Compact),
    ];
    prop::collection::vec(op, 0..=2000)
}

fn ops_strategy_zst() -> impl Strategy<Value = Vec<Op<()>>> {
    let key = key_strategy();
    let op = prop_oneof![
        50 => key.clone().prop_map(|k| Op::Insert(k, ())),
        25 => key.clone().prop_map(Op::Remove),
        24 => key.clone().prop_map(Op::Get),
        1 => Just(Op::Compact),
    ];
    prop::collection::vec(op, 0..=2000)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        max_shrink_iters: 50_000,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_equivalence_u64(ops in ops_strategy_u64()) {
        let mut t: HotTree<u64> = HotTree::new();
        let mut m: BTreeMap<Vec<u8>, u64> = BTreeMap::new();

        for op in ops {
            match op {
                Op::Insert(key, value) => {
                    let old_t = t.insert(&key, value);
                    let old_m = m.insert(key, value);
                    prop_assert_eq!(old_t, old_m);
                }
                Op::Remove(key) => {
                    let old_t = t.remove(&key);
                    let old_m = m.remove(key.as_slice());
                    prop_assert_eq!(old_t, old_m);
                }
                Op::Get(key) => {
                    let got_t = t.get(&key).copied();
                    let got_m = m.get(key.as_slice()).copied();
                    prop_assert_eq!(got_t, got_m);
                }
                Op::Compact => {
                    t.compact();
                }
            }

            prop_assert_eq!(t.len(), m.len());
        }

        validate_tree(&t);
        let got: Vec<(Vec<u8>, u64)> = t.iter().map(|(k, v)| (k, *v)).collect();
        let expected: Vec<(Vec<u8>, u64)> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
        prop_assert_eq!(got, expected);
    }

    #[test]
    fn prop_equivalence_zst(ops in ops_strategy_zst()) {
        let mut t: HotTree<()> = HotTree::new();
        let mut m: BTreeMap<Vec<u8>, ()> = BTreeMap::new();

        for op in ops {
            match op {
                Op::Insert(key, value) => {
                    let old_t = t.insert(&key, value);
                    let old_m = m.insert(key, ());
                    prop_assert_eq!(old_t, old_m);
                }
                Op::Remove(key) => {
                    let old_t = t.remove(&key);
                    let old_m = m.remove(key.as_slice());
                    prop_assert_eq!(old_t, old_m);
                }
                Op::Get(key) => {
                    let got_t = t.contains_key(&key);
                    let got_m = m.contains_key(key.as_slice());
                    prop_assert_eq!(got_t, got_m);
                }
                Op::Compact => {
                    t.compact();
                }
            }

            prop_assert_eq!(t.len(), m.len());
        }

        validate_tree(&t);
        let got: Vec<Vec<u8>> = t.iter().map(|(k, _)| k).collect();
        let expected: Vec<Vec<u8>> = m.keys().cloned().collect();
        prop_assert_eq!(got, expected);
    }
}

fn for_each_permutation<T: Clone>(items: &[T], mut f: impl FnMut(Vec<T>)) {
    fn rec<T: Clone>(items: &[T], used: &mut [bool], out: &mut Vec<T>, f: &mut impl FnMut(Vec<T>)) {
        if out.len() == items.len() {
            f(out.clone());
            return;
        }
        for i in 0..items.len() {
            if used[i] {
                continue;
            }
            used[i] = true;
            out.push(items[i].clone());
            rec(items, used, out, f);
            out.pop();
            used[i] = false;
        }
    }

    let mut used = vec![false; items.len()];
    let mut out = Vec::with_capacity(items.len());
    rec(items, &mut used, &mut out, &mut f);
}

#[test]
fn exhaustive_insert_order_small_set() {
    let keys: Vec<Vec<u8>> = vec![
        b"a".to_vec(),
        b"b".to_vec(),
        b"c".to_vec(),
        b"aa".to_vec(),
        b"ab".to_vec(),
        b"ba".to_vec(),
    ];

    for_each_permutation(&keys, |perm| {
        let mut t: HotTree<u64> = HotTree::new();
        let mut m: BTreeMap<Vec<u8>, u64> = BTreeMap::new();

        for (i, k) in perm.into_iter().enumerate() {
            let v = i as u64;
            assert_eq!(t.insert(&k, v), m.insert(k, v));
        }

        validate_tree(&t);
        let got: Vec<(Vec<u8>, u64)> = t.iter().map(|(k, v)| (k, *v)).collect();
        let expected: Vec<(Vec<u8>, u64)> = m.iter().map(|(k, v)| (k.clone(), *v)).collect();
        assert_eq!(got, expected);
    });
}

#[test]
fn exhaustive_remove_order_small_set() {
    let keys: Vec<Vec<u8>> = vec![
        b"a".to_vec(),
        b"b".to_vec(),
        b"c".to_vec(),
        b"aa".to_vec(),
        b"ab".to_vec(),
        b"ba".to_vec(),
    ];

    // Insert in a fixed order, then remove in all permutations.
    let mut base_tree: HotTree<u64> = HotTree::new();
    let mut base_map: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        let v = i as u64;
        assert_eq!(base_tree.insert(k, v), base_map.insert(k.clone(), v));
    }

    for_each_permutation(&keys, |perm| {
        let mut t = base_tree.clone();
        let mut m = base_map.clone();

        for k in perm {
            assert_eq!(t.remove(&k), m.remove(k.as_slice()));
            assert_eq!(t.len(), m.len());
            validate_tree(&t);
        }
        assert_eq!(t.len(), 0);
        assert!(t.root.is_null());
    });
}
