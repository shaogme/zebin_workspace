use zebin::io::SliceSerializer;
use zebin::{
    ZebinAccess, ZebinDeserialize, ZebinError, ZebinSerialize, access, deserialize, writer,
};

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
enum UnitMode {
    Idle,
    Busy,
}

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
enum TuplePacket {
    Empty,
    #[zebin(schema_key = 573785173)]
    Data(#[zebin(id = 0)] u32, #[zebin(id = 1)] String),
}

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
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

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
enum RecursiveNode {
    Leaf,
    Branch { children: Vec<RecursiveNode> },
}

#[test]
fn test_unit_enum_round_trip() {
    let idle = UnitMode::Idle;
    assert!(matches!(idle, UnitMode::Idle));
    let value = UnitMode::Busy;

    let mut buf = [0u8; 64];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<UnitMode, _>(&mut serializer).unwrap();
    writer_obj.write_all(value).unwrap();
    let written = serializer.written();

    let archived = access::<UnitMode, _>(&buf[..written]).unwrap();
    assert!(archived.is_busy());
    assert!(!archived.is_idle());
    assert_eq!(archived.tag(), 1);
}

#[cfg(feature = "alloc")]
#[test]
fn test_tuple_enum_round_trip() {
    let empty = TuplePacket::Empty;
    assert!(matches!(empty, TuplePacket::Empty));
    let value = TuplePacket::Data(7, "packet".to_string());
    let buf = zebin::serialize(value).unwrap();
    let archived = access::<TuplePacket, _>(&buf).unwrap();
    assert!(!archived.is_empty());
    assert_eq!(archived.tag(), 1);

    // TuplePacket::Data variant has a schema_key, so we can access it directly as a View
    let data = archived.as_data().expect("Should be Data variant");

    assert_eq!(*data.field0().unwrap(), 7);
    assert_eq!(unsafe { data.field1().unwrap().as_str() }, "packet");
}

#[cfg(feature = "alloc")]
#[test]
fn test_struct_enum_round_trip() {
    let ping = StructPacket::Ping;
    assert!(matches!(ping, StructPacket::Ping));
    let value = StructPacket::Data {
        code: 42,
        label: "hello".to_string(),
    };
    let buf = zebin::serialize(value).unwrap();
    let archived = access::<StructPacket, _>(&buf).unwrap();
    assert_eq!(archived.tag(), 1);

    // StructPacket::Data variant has a schema_key
    let data = archived.as_data().expect("Should be Data variant");

    assert_eq!(*data.code().unwrap(), 42);
    assert_eq!(unsafe { data.label().unwrap().as_str() }, "hello");
}

#[test]
fn test_invalid_enum_discriminant() {
    let value = UnitMode::Idle;

    let mut buf = [0u8; 64];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<UnitMode, _>(&mut serializer).unwrap();
    writer_obj.write_all(value).unwrap();
    let written = serializer.written();

    let root_offset = 4usize;
    buf[root_offset..root_offset + 4].copy_from_slice(&99u32.to_le_bytes());

    let err = zebin::validate::<UnitMode, _>(&buf[..written]).unwrap_err();
    match err {
        ZebinError::Access(zebin::error::AccessError::ValidationError { .. }) => {}
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[cfg(feature = "alloc")]
#[test]
fn test_truncated_enum_payload_rejected() {
    let value = TuplePacket::Data(11, "cut".to_string());
    let mut buf = zebin::serialize(value).unwrap();
    buf.pop();

    assert!(zebin::validate::<TuplePacket, _>(&buf).is_err());
}

#[cfg(feature = "alloc")]
#[test]
fn test_recursive_enum_depth_limit() {
    let mut current = RecursiveNode::Leaf;
    for _ in 0..300 {
        current = RecursiveNode::Branch {
            children: vec![current],
        };
    }

    let buf = zebin::serialize(current).unwrap();
    let err = zebin::validate::<RecursiveNode, _>(&buf).unwrap_err();
    assert!(matches!(
        err,
        ZebinError::Access(zebin::error::AccessError::RecursionLimitExceeded)
    ));
}

#[cfg(feature = "alloc")]
#[test]
fn test_enum_layout_mismatch_rejected() {
    let value = StructPacket::Data {
        code: 7,
        label: "layout".to_string(),
    };
    let mut buf = zebin::serialize(value).unwrap();

    let object_pos = 4 + 4;
    let label_encoding_pos = object_pos + 12 + 8 + 2;

    buf[label_encoding_pos] = zebin::schema::FieldEncoding::Fixed as u8;

    let err = zebin::validate::<StructPacket, _>(&buf).unwrap_err();
    assert!(matches!(
        err,
        ZebinError::Access(zebin::error::AccessError::UnexpectedFieldEncoding { .. })
    ));
}

#[test]
fn test_unit_enum_deserialize() {
    let value = UnitMode::Busy;
    let mut buf = [0u8; 64];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<UnitMode, _>(&mut serializer).unwrap();
    writer_obj.write_all(value.clone()).unwrap();
    let written = serializer.written();

    let deserialized: UnitMode = deserialize::<UnitMode, _>(&buf[..written]).unwrap();
    assert_eq!(deserialized, value);
}

#[cfg(feature = "alloc")]
#[test]
fn test_tuple_enum_deserialize() {
    let value = TuplePacket::Data(7, "packet".to_string());
    let buf = zebin::serialize(value.clone()).unwrap();
    let deserialized: TuplePacket = deserialize::<TuplePacket, _>(&buf).unwrap();
    assert_eq!(deserialized, value);
}

#[cfg(feature = "alloc")]
#[test]
fn test_struct_enum_deserialize() {
    let value = StructPacket::Data {
        code: 42,
        label: "hello".to_string(),
    };
    let buf = zebin::serialize(value.clone()).unwrap();
    let deserialized: StructPacket = deserialize::<StructPacket, _>(&buf).unwrap();
    assert_eq!(deserialized, value);
}

#[cfg(feature = "std")]
#[test]
fn test_enum_std_io_deserialize() {
    use std::io::Cursor;
    let value = UnitMode::Busy;
    let mut write_buf = Vec::new();
    zebin::prelude::serialize_to(&value, &mut write_buf).unwrap();
    let read_stream = Cursor::new(write_buf);
    let deserialized: UnitMode = zebin::prelude::deserialize_from(read_stream).unwrap();
    assert_eq!(deserialized, value);
}
