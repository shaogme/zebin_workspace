use zebin::{
    DecodeError, ValidationConfig, ValidationPathStack, ZebinArchive, ZebinError, ZebinSerialize,
    validate_detailed, validate_with_config,
};

#[derive(ZebinArchive, ZebinSerialize)]
struct Child {
    flag: bool,
}

#[derive(ZebinArchive, ZebinSerialize)]
struct Parent {
    children: Vec<Child>,
}

#[derive(ZebinArchive, ZebinSerialize)]
struct Node {
    children: Vec<Node>,
}

#[test]
fn test_validate_detailed_reports_logical_path() {
    let value = Parent {
        children: vec![Child { flag: true }],
    };

    let mut buf = zebin::encode(&value).unwrap();
    // Corrupt bool value (neither 0 nor 1)
    buf[8] = 2;

    let mut stack = ValidationPathStack::new();
    let err = validate_detailed::<Parent>(&buf, &mut stack).unwrap_err();

    // The path should be written back to the provided stack
    assert_eq!(stack.to_string(), "children[0].flag");

    assert!(matches!(
        err,
        ZebinError::Decode(DecodeError::ValidationError {
            message: "Invalid bool value",
            ..
        })
    ));
}

#[test]
fn test_validate_with_config_uses_custom_depth_limit() {
    let mut current = Node { children: vec![] };
    for _ in 0..8 {
        current = Node {
            children: vec![current],
        };
    }

    let buf = zebin::encode(&current).unwrap();
    // Pass None as we don't need path tracking here
    let err =
        validate_with_config::<Node>(&buf, ValidationConfig { max_depth: 2 }, None).unwrap_err();

    assert!(matches!(
        err,
        ZebinError::Decode(DecodeError::RecursionLimitExceeded)
    ));
}
