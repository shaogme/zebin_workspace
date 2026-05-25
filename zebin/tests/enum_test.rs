use zebin::{ZebinArchive, ZebinEncode, ZebinError};

#[derive(ZebinArchive, ZebinEncode)]
enum UnitMode {
    Idle,
    Busy,
}

#[derive(ZebinArchive, ZebinEncode)]
enum TuplePacket {
    Empty,
    #[zebin(schema_key = 573785173)]
    Data(#[zebin(id = 0)] u32, #[zebin(id = 1)] String),
}

#[derive(ZebinArchive, ZebinEncode)]
enum StructPacket {
    Ping,
    #[zebin(schema_key = 1432778632)]
    Data {
        #[zebin(id = 0)]
        code: u32,
        #[zebin(id = 1)]
        label: String,
    },
}

#[derive(ZebinArchive, ZebinEncode)]
enum RecursiveNode {
    Leaf,
    Branch { children: Vec<RecursiveNode> },
}

#[test]
fn test_unit_enum_round_trip() {
    let idle = UnitMode::Idle;
    assert!(matches!(idle, UnitMode::Idle));
    let value = UnitMode::Busy;
    let buf = zebin::encode(&value).unwrap();
    let archived = zebin::reader::<UnitMode>(&buf).unwrap();
    assert!(archived.is_busy());
    assert!(!archived.is_idle());
    assert_eq!(archived.tag(), 1);
}

#[test]
fn test_tuple_enum_round_trip() {
    let empty = TuplePacket::Empty;
    assert!(matches!(empty, TuplePacket::Empty));
    let value = TuplePacket::Data(7, "packet".to_string());
    let buf = zebin::encode(&value).unwrap();
    let reader = zebin::reader::<TuplePacket>(&buf).unwrap();
    let archived = reader.root();
    assert!(!archived.is_empty());
    assert_eq!(archived.tag(), 1);

    // TuplePacket::Data variant has a schema_key, so we can access it directly as a View
    let data = reader.as_data().expect("Should be Data variant");

    assert_eq!(*data.field0().unwrap(), 7);
    assert_eq!(unsafe { data.field1().unwrap().as_str() }, "packet");
}

#[test]
fn test_struct_enum_round_trip() {
    let ping = StructPacket::Ping;
    assert!(matches!(ping, StructPacket::Ping));
    let value = StructPacket::Data {
        code: 42,
        label: "hello".to_string(),
    };
    let buf = zebin::encode(&value).unwrap();
    let reader = zebin::reader::<StructPacket>(&buf).unwrap();
    let archived = reader.root();
    assert_eq!(archived.tag(), 1);

    // StructPacket::Data variant has a schema_key
    let data = reader.as_data().expect("Should be Data variant");

    assert_eq!(*data.code().unwrap(), 42);
    assert_eq!(unsafe { data.label().unwrap().as_str() }, "hello");
}

#[test]
fn test_invalid_enum_discriminant() {
    let value = UnitMode::Idle;
    let mut buf = zebin::encode(&value).unwrap();
    let root_offset = 4usize;
    buf[root_offset..root_offset + 4].copy_from_slice(&99u32.to_le_bytes());

    let err = zebin::validate::<UnitMode>(&buf).unwrap_err();
    match err {
        ZebinError::Decode(zebin::DecodeError::ValidationError { .. }) => {}
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[test]
fn test_truncated_enum_payload_rejected() {
    let value = TuplePacket::Data(11, "cut".to_string());
    let mut buf = zebin::encode(&value).unwrap();
    buf.pop();

    assert!(zebin::validate::<TuplePacket>(&buf).is_err());
}

#[test]
fn test_recursive_enum_depth_limit() {
    let mut current = RecursiveNode::Leaf;
    for _ in 0..300 {
        current = RecursiveNode::Branch {
            children: vec![current],
        };
    }

    let buf = zebin::encode(&current).unwrap();
    let err = zebin::validate::<RecursiveNode>(&buf).unwrap_err();
    assert!(matches!(
        err,
        ZebinError::Decode(zebin::DecodeError::RecursionLimitExceeded)
    ));
}

#[test]
fn test_enum_layout_mismatch_rejected() {
    let value = StructPacket::Data {
        code: 7,
        label: "layout".to_string(),
    };
    let mut buf = zebin::encode(&value).unwrap();

    let object_pos = 4 + 4;
    let label_encoding_pos = object_pos + 12 + 8 + 4 + 2;

    buf[label_encoding_pos] = zebin::FieldEncoding::Fixed as u8;

    let err = zebin::validate::<StructPacket>(&buf).unwrap_err();
    assert!(matches!(
        err,
        ZebinError::Decode(zebin::DecodeError::UnexpectedFieldEncoding { .. })
    ));
}
