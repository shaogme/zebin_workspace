#![cfg(feature = "alloc")]

use zebin::{ZebinAccess, ZebinDeserialize, ZebinError, ZebinSerialize};

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
pub struct Node {
    pub children: Vec<Node>,
}

#[test]
fn test_recursion_limit() {
    // Create a deeply nested structure
    // Depth 1: Root -> Vec[Node]
    // Depth 2: Node -> Vec[Node]
    // ...

    let mut current = Node { children: vec![] };
    for _ in 0..300 {
        current = Node {
            children: vec![current],
        };
    }

    let buf = zebin::serialize(current.clone()).unwrap();

    // Validation should fail due to recursion limit (default 256)
    let result = zebin::validate::<Node, _>(&buf);
    match result {
        Err(ZebinError::Access(zebin::error::AccessError::RecursionLimitExceeded)) => {}
        Err(e) => panic!("Expected RecursionLimitExceeded, got {:?}", e),
        Ok(_) => panic!("Expected error, got Ok"),
    }
}

use zebin::archive::IterArchive;

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
pub struct IterNode {
    pub children: IterArchive<Vec<IterNode>, IterNode>,
}

#[test]
fn test_recursion_limit_iter() {
    let mut current = IterNode {
        children: IterArchive::new(vec![]),
    };
    for _ in 0..300 {
        current = IterNode {
            children: IterArchive::new(vec![current]),
        };
    }

    let buf = zebin::serialize(current.clone()).unwrap();

    let result = zebin::validate::<IterNode, _>(&buf);
    match result {
        Err(ZebinError::Access(zebin::error::AccessError::RecursionLimitExceeded)) => {}
        Err(e) => panic!("Expected RecursionLimitExceeded, got {:?}", e),
        Ok(_) => panic!("Expected error, got Ok"),
    }
}
