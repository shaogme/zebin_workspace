use zebin::{ZebinAccess, ZebinDeserialize, ZebinSerialize};

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
#[zebin(schema_key = 324478056)]
#[zebin(revision = 3)]
pub struct VersionedUser {
    #[zebin(id = 1)]
    pub id: u64,
    #[zebin(id = 2)]
    pub age: u32,
    #[zebin(id = 0)]
    pub name: String,
}

#[cfg(feature = "alloc")]
#[test]
fn test_vtable_generation() {
    let user = VersionedUser {
        id: 101,
        age: 30,
        name: "Bob".to_string(),
    };
    let buf = zebin::serialize(user).unwrap();

    let object_pos = 4;
    let stable_schema_key = u32::from_le_bytes(buf[object_pos..object_pos + 4].try_into().unwrap());
    assert_eq!(stable_schema_key, 324478056);
    let schema_revision =
        u32::from_le_bytes(buf[object_pos + 4..object_pos + 8].try_into().unwrap());
    assert_eq!(schema_revision, 3);

    let num_fields = u16::from_le_bytes(buf[object_pos + 8..object_pos + 10].try_into().unwrap());
    assert_eq!(num_fields, 3);

    // Locate the field table (starts immediately after the 12-byte header)
    let table_pos = object_pos + 12;

    let field0_id = u16::from_le_bytes(buf[table_pos..table_pos + 2].try_into().unwrap());
    let field0_len =
        u32::from_le_bytes(buf[table_pos + 4..table_pos + 8].try_into().unwrap()) as usize;

    let field1_pos = table_pos + 8;
    let field1_id = u16::from_le_bytes(buf[field1_pos..field1_pos + 2].try_into().unwrap());
    let field1_len =
        u32::from_le_bytes(buf[field1_pos + 4..field1_pos + 8].try_into().unwrap()) as usize;

    let field2_pos = field1_pos + 8;
    let field2_id = u16::from_le_bytes(buf[field2_pos..field2_pos + 2].try_into().unwrap());
    let field2_len =
        u32::from_le_bytes(buf[field2_pos + 4..field2_pos + 8].try_into().unwrap()) as usize;

    assert_eq!(field0_id, 0);
    assert_eq!(field0_len, 7); // String "Bob" (4 bytes len + 3 bytes chars)

    assert_eq!(field1_id, 1);
    assert_eq!(field1_len, 8); // u64 id (8 bytes)

    assert_eq!(field2_id, 2);
    assert_eq!(field2_len, 4); // u32 age (4 bytes)

    let expected_len = 12 + 3 * 8 + (7 + 8 + 4);
    assert_eq!(buf.len() - object_pos, expected_len);
}

#[cfg(feature = "alloc")]
#[test]
fn test_safe_access() {
    let user = VersionedUser {
        id: 101,
        age: 30,
        name: "Bob".to_string(),
    };
    let buf = zebin::serialize(user).unwrap();

    let reader = zebin::access::<VersionedUser, _>(&buf).expect("Failed to validate archive");
    assert_eq!(reader.stable_schema_key(), 324478056);
    assert_eq!(reader.id().unwrap(), &101);
    assert_eq!(reader.age().unwrap(), &30);
    unsafe {
        assert_eq!(reader.name().unwrap().as_str(), "Bob");
    }
}

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
#[zebin(schema_key = 987654321)]
#[zebin(revision = 5)]
pub struct VersionedSensor {
    #[zebin(id = 1)]
    pub id: u64,
    #[zebin(id = 2)]
    pub value: u32,
}

#[test]
fn test_vtable_generation_no_alloc() {
    let sensor = VersionedSensor { id: 101, value: 30 };
    let mut buf = [0u8; 128];
    let mut serializer = zebin::io::SliceSerializer::new(&mut buf, 0);
    let mut writer = zebin::writer::<VersionedSensor, _>(&mut serializer).unwrap();
    writer.write_all(sensor).unwrap();
    let written = serializer.written();

    let object_pos = 4;
    let stable_schema_key = u32::from_le_bytes(buf[object_pos..object_pos + 4].try_into().unwrap());
    assert_eq!(stable_schema_key, 987654321);
    let schema_revision =
        u32::from_le_bytes(buf[object_pos + 4..object_pos + 8].try_into().unwrap());
    assert_eq!(schema_revision, 5);

    let num_fields = u16::from_le_bytes(buf[object_pos + 8..object_pos + 10].try_into().unwrap());
    assert_eq!(num_fields, 2);

    // Locate the field table (starts immediately after the 12-byte header)
    let table_pos = object_pos + 12;

    let field0_id = u16::from_le_bytes(buf[table_pos..table_pos + 2].try_into().unwrap());
    let field0_len =
        u32::from_le_bytes(buf[table_pos + 4..table_pos + 8].try_into().unwrap()) as usize;

    let field1_pos = table_pos + 8;
    let field1_id = u16::from_le_bytes(buf[field1_pos..field1_pos + 2].try_into().unwrap());
    let field1_len =
        u32::from_le_bytes(buf[field1_pos + 4..field1_pos + 8].try_into().unwrap()) as usize;

    assert_eq!(field0_id, 1);
    assert_eq!(field0_len, 8); // u64 id (8 bytes)

    assert_eq!(field1_id, 2);
    assert_eq!(field1_len, 4); // u32 value (4 bytes)

    let expected_len = 12 + 2 * 8 + (8 + 4);
    assert_eq!(written - object_pos, expected_len);
}

#[test]
fn test_safe_access_no_alloc() {
    let sensor = VersionedSensor { id: 101, value: 30 };
    let mut buf = [0u8; 128];
    let mut serializer = zebin::io::SliceSerializer::new(&mut buf, 0);
    let mut writer = zebin::writer::<VersionedSensor, _>(&mut serializer).unwrap();
    writer.write_all(sensor).unwrap();
    let written = serializer.written();

    let slice = &buf[..written];
    let reader = zebin::access::<VersionedSensor, _>(&slice).expect("Failed to validate archive");
    assert_eq!(reader.stable_schema_key(), 987654321);
    assert_eq!(reader.id().unwrap(), &101);
    assert_eq!(reader.value().unwrap(), &30);
}
