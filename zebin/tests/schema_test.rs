use zebin::{SchemaAware, ZebinArchive, ZebinSerialize};

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

    // Header should have non-zero layout offset
    let layout_offset = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    assert!(layout_offset > 0);
    assert!(layout_offset < buf.len() as u32);

    // Layout section should be at the end
    let num_layouts = u32::from_le_bytes(
        buf[layout_offset as usize..layout_offset as usize + 4]
            .try_into()
            .unwrap(),
    );
    assert_eq!(num_layouts, 1);

    let layout0_offset = u32::from_le_bytes(
        buf[layout_offset as usize + 4..layout_offset as usize + 8]
            .try_into()
            .unwrap(),
    );
    let layout0_pos = (layout_offset + layout0_offset) as usize;

    // Layout entry: stable_schema_key (4) + schema_revision (4) + field_count (2) + encoding/reserved (6) + 3 field records.
    let stable_schema_key =
        u32::from_le_bytes(buf[layout0_pos..layout0_pos + 4].try_into().unwrap());
    assert_eq!(stable_schema_key, 324478056);
    let schema_revision =
        u32::from_le_bytes(buf[layout0_pos + 4..layout0_pos + 8].try_into().unwrap());
    assert_eq!(schema_revision, 3);

    let num_fields = u16::from_le_bytes(buf[layout0_pos + 8..layout0_pos + 10].try_into().unwrap());
    assert_eq!(num_fields, 3);

    let field0_id = u16::from_le_bytes(buf[layout0_pos + 16..layout0_pos + 18].try_into().unwrap());
    let field0_offset =
        u32::from_le_bytes(buf[layout0_pos + 18..layout0_pos + 22].try_into().unwrap());
    let field1_id = u16::from_le_bytes(buf[layout0_pos + 24..layout0_pos + 26].try_into().unwrap());
    let field1_offset =
        u32::from_le_bytes(buf[layout0_pos + 26..layout0_pos + 30].try_into().unwrap());
    let field2_id = u16::from_le_bytes(buf[layout0_pos + 32..layout0_pos + 34].try_into().unwrap());
    let field2_offset =
        u32::from_le_bytes(buf[layout0_pos + 34..layout0_pos + 38].try_into().unwrap());

    use zebin::memoffset::offset_of;
    assert_eq!(field0_id, 0);
    assert_eq!(
        field0_offset as usize,
        offset_of!(ArchivedVersionedUser, name)
    );
    assert_eq!(field1_id, 1);
    assert_eq!(
        field1_offset as usize,
        offset_of!(ArchivedVersionedUser, id)
    );
    assert_eq!(field2_id, 2);
    assert_eq!(
        field2_offset as usize,
        offset_of!(ArchivedVersionedUser, age)
    );
}

#[test]
fn test_safe_access() {
    let user = VersionedUser {
        id: 101,
        age: 30,
        name: "Bob".to_string(),
    };
    let buf = zebin::encode(&user).unwrap();

    let reader = zebin::decode::<VersionedUser>(&buf).expect("Failed to validate archive");
    assert_eq!(reader.stable_schema_key(), 324478056);
    assert_eq!(reader.id().unwrap(), &101);
    assert_eq!(reader.age().unwrap(), &30);
    unsafe {
        assert_eq!(reader.name().unwrap().as_str(), "Bob");
    }
}
