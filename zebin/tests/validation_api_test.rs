use zebin::{ZebinAccess, ZebinDeserialize, ZebinSerialize};

#[cfg(feature = "alloc")]
use zebin::prelude::{
    AccessError, ValidationConfig, ValidationPathStack, ZebinError, validate_detailed,
    validate_with_config,
};

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
struct Child {
    flag: bool,
}

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
struct Parent {
    children: Vec<Child>,
}

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
struct Node {
    children: Vec<Node>,
}

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
#[zebin(schema_key = 0x5151)]
struct SchemaRecord {
    #[zebin(id = 1)]
    flag: bool,
    #[zebin(id = 2)]
    name: String,
}

#[cfg(feature = "alloc")]
#[test]
fn test_validate_detailed_reports_logical_path() {
    let value = Parent {
        children: vec![Child { flag: true }],
    };

    let mut buf = zebin::serialize(value).unwrap();
    // Corrupt bool value (neither 0 nor 1)
    buf[5] = 2;

    let mut stack = ValidationPathStack::new();
    let err = validate_detailed::<Parent, _>(&buf, &mut stack).unwrap_err();

    // The path should be written back to the provided stack
    assert_eq!(stack.to_string(), "children[0].flag");

    assert!(matches!(
        err,
        ZebinError::Access(AccessError::ValidationError {
            message: "Invalid bool value",
            ..
        })
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_validate_with_config_uses_custom_depth_limit() {
    let mut current = Node { children: vec![] };
    for _ in 0..8 {
        current = Node {
            children: vec![current],
        };
    }

    let buf = zebin::serialize(current).unwrap();
    // Pass None as we don't need path tracking here
    let err = validate_with_config::<Node, _>(
        &buf,
        ValidationConfig {
            max_depth: 2,
            ..ValidationConfig::default()
        },
        None,
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ZebinError::Access(AccessError::RecursionLimitExceeded)
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_validate_detailed_reports_schema_field_encoding_path() {
    let value = SchemaRecord {
        flag: true,
        name: "Ada".to_string(),
    };

    let mut buf = zebin::serialize(value).unwrap();
    let object_pos = 4;
    let first_entry_encoding_pos = object_pos + 12 + 2;
    buf[first_entry_encoding_pos] = zebin::schema::FieldEncoding::LengthPrefixed as u8;

    let mut stack = ValidationPathStack::new();
    let err = validate_detailed::<SchemaRecord, _>(&buf, &mut stack).unwrap_err();

    assert_eq!(stack.to_string(), "flag");
    assert!(matches!(
        err,
        ZebinError::Access(AccessError::UnexpectedFieldEncoding { field_id: 1, .. })
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_validate_detailed_reports_schema_field_length_path() {
    let value = SchemaRecord {
        flag: true,
        name: "Ada".to_string(),
    };

    let mut buf = zebin::serialize(value).unwrap();
    let object_pos = 4;
    let first_entry_payload_len_pos = object_pos + 12 + 4;
    buf[first_entry_payload_len_pos..first_entry_payload_len_pos + 4]
        .copy_from_slice(&2u32.to_le_bytes());

    let mut stack = ValidationPathStack::new();
    let err = validate_detailed::<SchemaRecord, _>(&buf, &mut stack).unwrap_err();

    assert_eq!(stack.to_string(), "flag");
    assert!(matches!(
        err,
        ZebinError::Access(AccessError::FieldLengthMismatch { field_id: 1, .. })
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_validate_detailed_reports_duplicate_schema_field_path() {
    let value = SchemaRecord {
        flag: true,
        name: "Ada".to_string(),
    };

    let mut buf = zebin::serialize(value).unwrap();
    let object_pos = 4;
    // In Forward Field Table, the second entry in the field table is immediately after the first entry.
    let second_entry_id_pos = object_pos + 12 + zebin::schema::FieldEntry::SIZE;
    buf[second_entry_id_pos..second_entry_id_pos + 2].copy_from_slice(&1u16.to_le_bytes());
    let second_entry_encoding_pos = second_entry_id_pos + 2;
    buf[second_entry_encoding_pos] = zebin::schema::FieldEncoding::Fixed as u8;
    let second_entry_payload_len_pos = second_entry_id_pos + 4;
    buf[second_entry_payload_len_pos..second_entry_payload_len_pos + 4]
        .copy_from_slice(&1u32.to_le_bytes());

    let mut stack = ValidationPathStack::new();
    let err = validate_detailed::<SchemaRecord, _>(&buf, &mut stack).unwrap_err();

    assert_eq!(stack.to_string(), "flag");
    assert!(matches!(
        err,
        ZebinError::Access(AccessError::DuplicateField { field_id: 1, .. })
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_validate_detailed_reports_trailing_bytes_at_root() {
    let value = Parent {
        children: vec![Child { flag: true }],
    };

    let mut buf = zebin::serialize(value).unwrap();
    buf.push(0);

    let mut stack = ValidationPathStack::new();
    let err = validate_detailed::<Parent, _>(&buf, &mut stack).unwrap_err();

    assert_eq!(stack.to_string(), "<root>");
    assert!(matches!(
        err,
        ZebinError::Access(AccessError::ValidationError {
            message: "Trailing bytes after root object",
            ..
        })
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_reader_rejects_invalid_sequence_marker_before_building_view() {
    let value = Parent {
        children: vec![Child { flag: true }],
    };

    let mut buf = zebin::serialize(value).unwrap();
    buf[4] = 2;

    let mut reader_obj = zebin::reader::<Parent, _>(&buf).unwrap();
    let err = match reader_obj.read() {
        Ok(_) => panic!("reader accepted invalid sequence marker"),
        Err(error) => error,
    };

    assert!(matches!(
        err,
        ZebinError::Access(AccessError::ValidationError {
            message: "Invalid sequence marker",
            ..
        })
    ));
}
