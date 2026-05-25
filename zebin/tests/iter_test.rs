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
    let bytes = zebin::encode(&wrapped).expect("failed to encode");

    let decoded: Vec<u64> = zebin::decode::<Vec<u64>>(&bytes).expect("failed to decode");
    assert_eq!(decoded, vec![10, 20, 30]);
}

#[test]
fn test_iter_archive_hashset() {
    let mut set = HashSet::new();
    set.insert(42u32);
    set.insert(100u32);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::encode(&wrapped).expect("failed to encode");

    let mut decoded: Vec<u32> = zebin::decode::<Vec<u32>>(&bytes).expect("failed to decode");
    decoded.sort();
    assert_eq!(decoded, vec![42, 100]);
}

#[test]
fn test_iter_archive_lazy() {
    let mut set = BTreeSet::new();
    set.insert(100u64);
    set.insert(200u64);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::encode(&wrapped).expect("failed to encode");

    // 不做 Restore，直接获取零拷贝延迟反序列化视图
    let reader =
        zebin::reader::<IterArchive<BTreeSet<u64>, u64>>(&bytes).expect("failed to create reader");
    let archived_iter = reader.root();

    assert_eq!(archived_iter.len(), 2);

    let mut iter = archived_iter.iter();
    assert_eq!(iter.next().unwrap().unwrap(), 100);
    assert_eq!(iter.next().unwrap().unwrap(), 200);
    assert!(iter.next().is_none());
}

#[test]
fn test_iter_archive_restore_explicit() {
    let mut set = BTreeSet::new();
    set.insert(100u64);
    set.insert(200u64);

    let wrapped = IterArchive::new(set);
    let bytes = zebin::encode(&wrapped).expect("failed to encode");

    let reader =
        zebin::reader::<IterArchive<BTreeSet<u64>, u64>>(&bytes).expect("failed to create reader");
    let archived_iter = reader.root();

    use zebin::prelude::Restore;

    // 显式恢复为 Vec
    let restored_vec: Vec<u64> = archived_iter.restore().expect("failed to restore Vec");
    assert_eq!(restored_vec, vec![100, 200]);

    // 显式恢复为 VecDeque
    use std::collections::VecDeque;
    let restored_deque: VecDeque<u64> =
        archived_iter.restore().expect("failed to restore VecDeque");
    let expected_deque: VecDeque<u64> = vec![100, 200].into();
    assert_eq!(restored_deque, expected_deque);
}
