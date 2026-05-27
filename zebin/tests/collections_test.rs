#![cfg(feature = "alloc")]
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq, Clone)]
#[zebin(schema_key = 100)]
pub struct CollectionContainer {
    #[zebin(id = 0)]
    pub btree_set: BTreeSet<u32>,
    #[zebin(id = 1)]
    pub hash_set: Option<HashSet<String>>,
    #[zebin(id = 2)]
    pub btree_map: BTreeMap<String, u32>,
    #[zebin(id = 3)]
    pub hash_map: Option<HashMap<u32, String>>,
}

#[test]
fn test_btreeset_direct() {
    let mut set = BTreeSet::new();
    set.insert(10u32);
    set.insert(5u32);
    set.insert(20u32);

    let bytes = zebin::serialize(&set).expect("failed to serialize BTreeSet");
    let restored: BTreeSet<u32> =
        zebin::deserialize::<BTreeSet<u32>, _>(&bytes).expect("failed to deserialize BTreeSet");
    assert_eq!(restored, set);

    let archived = zebin::access::<BTreeSet<u32>, _>(&bytes).expect("failed to access BTreeSet");
    assert_eq!(archived.len(), 3);
}

#[test]
fn test_hashset_direct() {
    let mut set = HashSet::new();
    set.insert("hello".to_string());
    set.insert("world".to_string());

    let bytes = zebin::serialize(&set).expect("failed to serialize HashSet");
    let restored: HashSet<String> =
        zebin::deserialize::<HashSet<String>, _>(&bytes).expect("failed to deserialize HashSet");
    assert_eq!(restored, set);
}

#[test]
fn test_binary_heap_direct() {
    let mut heap = BinaryHeap::new();
    heap.push(10);
    heap.push(30);
    heap.push(20);

    let bytes = zebin::serialize(&heap).expect("failed to serialize BinaryHeap");
    let restored: BinaryHeap<i32> =
        zebin::deserialize::<BinaryHeap<i32>, _>(&bytes).expect("failed to deserialize BinaryHeap");

    let restored_sorted: Vec<i32> = restored.into_sorted_vec();
    let expected_sorted: Vec<i32> = heap.into_sorted_vec();
    assert_eq!(restored_sorted, expected_sorted);
}

#[test]
fn test_btreemap_direct() {
    let mut map = BTreeMap::new();
    map.insert("first".to_string(), 10u32);
    map.insert("second".to_string(), 20u32);

    let bytes = zebin::serialize(&map).expect("failed to serialize BTreeMap");
    let restored: BTreeMap<String, u32> = zebin::deserialize::<BTreeMap<String, u32>, _>(&bytes)
        .expect("failed to deserialize BTreeMap");
    assert_eq!(restored, map);
}

#[test]
fn test_hashmap_direct() {
    let mut map = HashMap::new();
    map.insert(1u32, "one".to_string());
    map.insert(2u32, "two".to_string());

    let bytes = zebin::serialize(&map).expect("failed to serialize HashMap");
    let restored: HashMap<u32, String> = zebin::deserialize::<HashMap<u32, String>, _>(&bytes)
        .expect("failed to deserialize HashMap");
    assert_eq!(restored, map);
}

#[test]
fn test_collections_container() {
    let mut btree_set = BTreeSet::new();
    btree_set.insert(1);
    btree_set.insert(2);

    let mut hash_set = HashSet::new();
    hash_set.insert("test".to_string());

    let mut btree_map = BTreeMap::new();
    btree_map.insert("key".to_string(), 42u32);

    let mut hash_map = HashMap::new();
    hash_map.insert(100u32, "value".to_string());

    let container = CollectionContainer {
        btree_set,
        hash_set: Some(hash_set),
        btree_map,
        hash_map: Some(hash_map),
    };

    let bytes = zebin::serialize(&container).expect("failed to serialize container");
    let restored: CollectionContainer = zebin::deserialize::<CollectionContainer, _>(&bytes)
        .expect("failed to deserialize container");
    assert_eq!(restored, container);
}
