use std::collections::BTreeMap;

use proptest::prelude::*;
use proptest_derive::Arbitrary;

use crate::HotTree;

/// Simple model implementation using BTreeMap for comparison
#[derive(Default, Clone)]
struct Model {
    map: BTreeMap<Vec<u8>, u64>,
}

impl Model {
    fn insert(&mut self, key: Vec<u8>, value: u64) -> Option<u64> {
        self.map.insert(key, value)
    }

    fn get(&self, key: &[u8]) -> Option<&u64> {
        self.map.get(key)
    }

    fn remove(&mut self, key: &[u8]) -> Option<u64> {
        self.map.remove(key)
    }

    fn len(&self) -> usize {
        self.map.len()
    }

    fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Actions to test against both implementations
#[derive(Arbitrary, Debug, Clone)]
enum Action {
    Insert(KeyValue),
    Get(Key),
    Remove(Key),
}

/// Wrapper for key generation with custom strategy
#[derive(Debug, Clone)]
struct Key(Vec<u8>);

/// Wrapper for key-value pair
#[derive(Debug, Clone)]
struct KeyValue {
    key: Key,
    value: u64,
}

impl Arbitrary for Key {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        prop_oneof![
            // Empty key
            Just(Key(vec![])),
            // Short keys (1-4 bytes)
            prop::collection::vec(any::<u8>(), 1..4).prop_map(Key),
            // Medium keys (4-64 bytes)
            prop::collection::vec(any::<u8>(), 4..64).prop_map(Key),
            // Keys with natural delimiters to exercise prefix compression
            "[a-z]{4,8}/[a-z]{1,8}".prop_map(|s| Key(s.into_bytes())),
            "[a-z]{4,8}:[a-z]{1,8}".prop_map(|s| Key(s.into_bytes())),
            "[a-z]{4,8}\\[a-z]{1,8}".prop_map(|s| Key(s.into_bytes())),
            // Keys with shared prefixes
            "[a-z]{4,8}".prop_map(|prefix| {
                let mut key = prefix.clone().into_bytes();
                key.extend_from_slice(b"/suffix");
                Key(key)
            }),
        ]
        .boxed()
    }
}

impl Arbitrary for KeyValue {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        (any::<Key>(), any::<u64>())
            .prop_map(|(key, value)| KeyValue { key, value })
            .boxed()
    }
}

/// Test harness that executes actions on both HotTree and Model
#[derive(Default)]
struct Test {
    tree: HotTree<u64>,
    model: Model,
}

impl Test {
    fn execute(&mut self, action: Action) {
        match action {
            Action::Insert(kv) => {
                let key = kv.key.0.clone();
                let tree_result = self.tree.insert(&key, kv.value);
                let model_result = self.model.insert(key.clone(), kv.value);
                assert_eq!(
                    tree_result, model_result,
                    "Insert mismatch: key={:?}, tree_result={:?}, model_result={:?}",
                    key, tree_result, model_result
                );
            }
            Action::Get(key) => {
                let key_bytes = key.0.clone();
                let tree_result = self.tree.get(&key_bytes);
                let model_result = self.model.get(&key_bytes);
                assert_eq!(
                    tree_result, model_result,
                    "Get mismatch: key={:?}, tree_result={:?}, model_result={:?}",
                    key_bytes, tree_result, model_result
                );
            }
            Action::Remove(key) => {
                let key_bytes = key.0.clone();
                let tree_result = self.tree.remove(&key_bytes);
                let model_result = self.model.remove(&key_bytes);
                assert_eq!(
                    tree_result, model_result,
                    "Remove mismatch: key={:?}, tree_result={:?}, model_result={:?}",
                    key_bytes, tree_result, model_result
                );
            }
        }
        // Always verify len matches
        assert_eq!(
            self.tree.len(),
            self.model.len(),
            "Length mismatch after action: tree={}, model={}",
            self.tree.len(),
            self.model.len()
        );
        assert_eq!(
            self.tree.is_empty(),
            self.model.is_empty(),
            "is_empty mismatch: tree={}, model={}",
            self.tree.is_empty(),
            self.model.is_empty()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn proptest_vs_btreemap(actions in prop::collection::vec(any::<Action>(), 1..64)) {
        let mut test = Test::default();
        for action in actions {
            test.execute(action);
        }
    }
}
