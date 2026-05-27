#![cfg(feature = "alloc")]
use std::collections::{BTreeSet, HashSet};
use zebin::archive::IterArchive;

#[test]
fn test_iter_archive_btreeset() {
    let mut set = BTreeSet::new();
    set.insert(10u64);
    set.insert(20u64);
    set.insert(30u64);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::serialize(wrapped).expect("failed to serialize");

    let deserialized: Vec<u64> =
        zebin::deserialize::<Vec<u64>, _>(&bytes).expect("failed to deserialize");
    assert_eq!(deserialized, vec![10, 20, 30]);
}

#[test]
fn test_iter_archive_hashset() {
    let mut set = HashSet::new();
    set.insert(42u32);
    set.insert(100u32);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::serialize(wrapped).expect("failed to serialize");

    let mut deserialized: Vec<u32> =
        zebin::deserialize::<Vec<u32>, _>(&bytes).expect("failed to deserialize");
    deserialized.sort();
    assert_eq!(deserialized, vec![42, 100]);
}

#[test]
fn test_iter_archive_lazy() {
    let mut set = BTreeSet::new();
    set.insert(100u64);
    set.insert(200u64);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::serialize(wrapped).expect("failed to serialize");

    // 不做 Deserialize，直接获取零拷贝延迟反序列化视图
    let mut reader = zebin::reader::<IterArchive<BTreeSet<u64>, u64>, _>(&bytes)
        .expect("failed to create reader");
    let archived_iter = reader.read().unwrap();

    assert_eq!(archived_iter.len(), 2);

    let mut iter = archived_iter.iter();
    assert_eq!(iter.next().unwrap().unwrap(), 100);
    assert_eq!(iter.next().unwrap().unwrap(), 200);
    assert!(iter.next().is_none());
}

#[test]
fn test_iter_archive_deserialize_explicit() {
    let mut set = BTreeSet::new();
    set.insert(100u64);
    set.insert(200u64);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::serialize(wrapped).expect("failed to serialize");

    let mut reader = zebin::reader::<IterArchive<BTreeSet<u64>, u64>, _>(&bytes)
        .expect("failed to create reader");
    let archived_iter = reader.read().unwrap();

    use zebin::prelude::Deserialize;

    // 显式恢复为 Vec
    let deserialized_vec: Vec<u64> = archived_iter
        .deserialize()
        .expect("failed to deserialize Vec");
    assert_eq!(deserialized_vec, vec![100, 200]);

    // 显式恢复为 VecDeque
    use std::collections::VecDeque;
    let deserialized_deque: VecDeque<u64> = archived_iter
        .deserialize()
        .expect("failed to deserialize VecDeque");
    let expected_deque: VecDeque<u64> = vec![100, 200].into();
    assert_eq!(deserialized_deque, expected_deque);
}

// ─── Chunked block index tests ─────────────────────────────────────────────

/// Serialize 200 u64 elements, verify get(i) returns correct value for all i.
#[test]
fn test_chunked_index_random_access() {
    let data: Vec<u64> = (0..200).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();

    assert_eq!(archived.len(), 200);

    // Verify every element through random access.
    for i in 0..200 {
        let val = archived.get(i).unwrap_or_else(|_| panic!("get({i})"));
        assert_eq!(val, i as u64, "mismatch at index {i}");
    }

    // Out-of-bounds must return an error.
    assert!(archived.get(200).is_err());
}

/// iter_from(start) should yield the remaining elements correctly.
#[test]
fn test_chunked_index_iter_from() {
    let data: Vec<u64> = (0..200).collect();
    let wrapped = IterArchive::new(data);
    let bytes = zebin::serialize(wrapped).expect("serialize");

    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();

    // Iterate from a block boundary.
    let from_64: Vec<u64> = archived.iter_from(64).map(|r| r.unwrap()).collect();
    assert_eq!(from_64.len(), 136);
    assert_eq!(from_64[0], 64);
    assert_eq!(*from_64.last().unwrap(), 199);

    // Iterate from a non-boundary offset inside a block.
    let from_100: Vec<u64> = archived.iter_from(100).map(|r| r.unwrap()).collect();
    assert_eq!(from_100.len(), 100);
    assert_eq!(from_100[0], 100);

    // iter_from beyond len → empty.
    let from_end: Vec<u64> = archived.iter_from(200).map(|r| r.unwrap()).collect();
    assert!(from_end.is_empty());
}

/// Sequences ≤ 64 elements should not contain a block index.
#[test]
fn test_small_sequence_no_index() {
    let data: Vec<u64> = (0..30).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    // Access as Vec (via ForwardSequenceStrategy) – should work.
    let deserialized: Vec<u64> =
        zebin::deserialize::<Vec<u64>, _>(&bytes).expect("deserialize Vec");
    assert_eq!(deserialized, data);

    // Access as IterArchive – should also work and still support get().
    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();
    assert_eq!(archived.len(), 30);
    // get() falls back to linear scan when no index is present.
    for i in 0..30 {
        assert_eq!(archived.get(i).unwrap(), i as u64);
    }
}

/// Boundary: exactly 64 elements (= chunk_size, should NOT produce an index
/// since element_count is not > chunk_size).
#[test]
fn test_chunked_index_boundary_64() {
    let data: Vec<u64> = (0..64).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();
    assert_eq!(archived.len(), 64);
    for i in 0..64 {
        assert_eq!(archived.get(i).unwrap(), i as u64);
    }
}

/// Boundary: exactly 65 elements (should produce an index with 2 blocks).
#[test]
fn test_chunked_index_boundary_65() {
    let data: Vec<u64> = (0..65).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();
    assert_eq!(archived.len(), 65);
    for i in 0..65 {
        assert_eq!(archived.get(i).unwrap(), i as u64, "mismatch at {i}");
    }
}

/// Boundary: exactly 128 elements (2 full blocks).
#[test]
fn test_chunked_index_boundary_128() {
    let data: Vec<u64> = (0..128).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();
    assert_eq!(archived.len(), 128);
    for i in 0..128 {
        assert_eq!(archived.get(i).unwrap(), i as u64);
    }
}

/// Verify that data serialized as IterArchive (which writes block index)
/// can still be deserialized as Vec<T> (which uses ForwardSequenceStrategy).
#[test]
fn test_backward_compat_iter_to_vec() {
    let data: Vec<u64> = (0..200).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    // Access as Vec<u64> – ForwardSequenceStrategy must skip block index.
    let deserialized: Vec<u64> = zebin::deserialize::<Vec<u64>, _>(&bytes).expect("deserialize");
    assert_eq!(deserialized, data);
}

/// Verify that data serialized as Vec<T> (no block index) can be deserialized
/// as IterArchive.
#[test]
fn test_backward_compat_vec_to_iter() {
    let data: Vec<u64> = (0..200).collect();
    let bytes = zebin::serialize(data.clone()).expect("serialize");

    // Access as IterArchive – should still work (no block index, linear fallback).
    let mut reader = zebin::reader::<IterArchive<Vec<u64>, u64>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();
    assert_eq!(archived.len(), 200);
    // get() should work via linear scan.
    assert_eq!(archived.get(0).unwrap(), 0);
    assert_eq!(archived.get(199).unwrap(), 199);
}

/// Test with variable-length elements (strings) to ensure the index
/// records correct offsets despite non-uniform element sizes.
#[test]
fn test_chunked_index_variable_length_elements() {
    let data: Vec<String> = (0..200).map(|i| format!("item_{:05}", i)).collect();
    let wrapped = IterArchive::new(data.clone());
    let bytes = zebin::serialize(wrapped).expect("serialize");

    let mut reader = zebin::reader::<IterArchive<Vec<String>, String>, _>(&bytes).expect("reader");
    let archived = reader.read().unwrap();
    assert_eq!(archived.len(), 200);

    // Spot-check several positions across block boundaries.
    for &i in &[0, 1, 63, 64, 65, 127, 128, 129, 190, 199] {
        let val: String = {
            use zebin::prelude::Deserialize;
            archived.get(i).unwrap().deserialize().unwrap()
        };
        assert_eq!(val, format!("item_{:05}", i), "mismatch at {i}");
    }
}
