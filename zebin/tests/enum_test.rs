use zebin::{ArchiveHeader, ArchiveHeaderTrait, ZebinArchive, ZebinError, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize)]
enum UnitMode {
    Idle,
    Busy,
}

#[derive(ZebinArchive, ZebinSerialize)]
enum TuplePacket {
    Empty,
    #[zebin(schema_key = 573785173)]
    Data(#[zebin(id = 0)] u32, #[zebin(id = 1)] String),
}

#[derive(ZebinArchive, ZebinSerialize)]
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

#[derive(ZebinArchive, ZebinSerialize)]
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
    let archived = zebin::decode::<UnitMode>(&buf).unwrap();
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
    let reader = zebin::decode::<TuplePacket>(&buf).unwrap();
    let archived = reader.root();
    assert!(!archived.is_empty());
    assert_eq!(archived.tag(), 1);

    // TuplePacket::Data variant has a schema_key, so we can access it directly as a View
    let data = reader.as_data().unwrap().expect("Should be Data variant");

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
    let reader = zebin::decode::<StructPacket>(&buf).unwrap();
    let archived = reader.root();
    assert_eq!(archived.tag(), 1);

    // StructPacket::Data variant has a schema_key
    let data = reader.as_data().unwrap().expect("Should be Data variant");

    assert_eq!(*data.code().unwrap(), 42);
    assert_eq!(unsafe { data.label().unwrap().as_str() }, "hello");
}

#[test]
fn test_invalid_enum_discriminant() {
    let value = UnitMode::Idle;
    let mut buf = zebin::encode(&value).unwrap();
    let header = ArchiveHeader::parse(&buf).unwrap();
    let root_offset = header.root_offset.get() as usize;
    buf[root_offset..root_offset + 4].copy_from_slice(&99u32.to_le_bytes());

    let err = zebin::validate::<UnitMode>(&buf).unwrap_err();
    match err {
        ZebinError::Access(zebin::ValidateError::ValidationError { .. }) => {}
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
        ZebinError::Access(zebin::ValidateError::RecursionLimitExceeded)
    ));
}

#[test]
fn test_enum_layout_mismatch_rejected() {
    let value = StructPacket::Data {
        code: 7,
        label: "layout".to_string(),
    };
    let mut buf = zebin::encode(&value).unwrap();

    let header = ArchiveHeader::parse(&buf).unwrap();
    let layout_offset = header.layout_offset.get() as usize;
    let layout0_offset = u32::from_le_bytes(
        buf[layout_offset + 4..layout_offset + 8]
            .try_into()
            .unwrap(),
    ) as usize;
    let layout0_pos = layout_offset + layout0_offset;
    let field0_offset_pos = layout0_pos + 16 + 2;
    let field0_offset = u32::from_le_bytes(
        buf[field0_offset_pos..field0_offset_pos + 4]
            .try_into()
            .unwrap(),
    );
    buf[field0_offset_pos..field0_offset_pos + 4]
        .copy_from_slice(&(field0_offset + 1).to_le_bytes());

    let err = zebin::validate::<StructPacket>(&buf).unwrap_err();
    assert!(matches!(
        err,
        ZebinError::Access(zebin::ValidateError::LayoutOffsetMismatch { .. })
    ));
}
