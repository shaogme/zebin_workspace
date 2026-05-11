use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize)]
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

#[test]
fn test_vtable_generation() {
    let user = VersionedUser {
        id: 101,
        age: 30,
        name: "Bob".to_string(),
    };
    let buf = zebin::encode(&user).unwrap();

    let object_pos = 4;
    let stable_schema_key = u32::from_le_bytes(buf[object_pos..object_pos + 4].try_into().unwrap());
    assert_eq!(stable_schema_key, 324478056);
    let schema_revision =
        u32::from_le_bytes(buf[object_pos + 4..object_pos + 8].try_into().unwrap());
    assert_eq!(schema_revision, 3);

    let num_fields = u16::from_le_bytes(buf[object_pos + 8..object_pos + 10].try_into().unwrap());
    assert_eq!(num_fields, 3);

    let table_pos = object_pos + 12;
    let field0_id = u16::from_le_bytes(buf[table_pos..table_pos + 2].try_into().unwrap());
    let field1_id = u16::from_le_bytes(buf[table_pos + 8..table_pos + 10].try_into().unwrap());
    let field2_id = u16::from_le_bytes(buf[table_pos + 16..table_pos + 18].try_into().unwrap());

    assert_eq!(field0_id, 0);
    assert_eq!(field1_id, 1);
    assert_eq!(field2_id, 2);
}

#[test]
fn test_safe_access() {
    let user = VersionedUser {
        id: 101,
        age: 30,
        name: "Bob".to_string(),
    };
    let buf = zebin::encode(&user).unwrap();

    let reader = zebin::reader::<VersionedUser>(&buf).expect("Failed to validate archive");
    assert_eq!(reader.stable_schema_key(), 324478056);
    assert_eq!(reader.id().unwrap(), &101);
    assert_eq!(reader.age().unwrap(), &30);
    unsafe {
        assert_eq!(reader.name().unwrap().as_str(), "Bob");
    }
}
