use zebin::{ZebinArchive, ZebinSerialize, ZebinError};

#[derive(ZebinArchive, ZebinSerialize)]
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

    let buf = zebin::encode(&current).unwrap();

    // Validation should fail due to recursion limit (default 256)
    let result = zebin::validate::<Node>(&buf);
    match result {
        Err(ZebinError::RecursionLimitExceeded) => {}
        Err(e) => panic!("Expected RecursionLimitExceeded, got {:?}", e),
        Ok(_) => panic!("Expected error, got Ok"),
    }
}
