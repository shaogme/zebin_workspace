use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize)]
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

    // Layout entry: schema_id (4) + field_count (2) + reserved (2) + 3 field records.
    let schema_id = u32::from_le_bytes(buf[layout0_pos..layout0_pos + 4].try_into().unwrap());
    assert_eq!(schema_id, 0);

    let num_fields = u16::from_le_bytes(buf[layout0_pos + 4..layout0_pos + 6].try_into().unwrap());
    assert_eq!(num_fields, 3);

    let field0_id = u16::from_le_bytes(buf[layout0_pos + 8..layout0_pos + 10].try_into().unwrap());
    let field0_offset =
        u16::from_le_bytes(buf[layout0_pos + 10..layout0_pos + 12].try_into().unwrap());
    let field1_id =
        u16::from_le_bytes(buf[layout0_pos + 12..layout0_pos + 14].try_into().unwrap());
    let field1_offset =
        u16::from_le_bytes(buf[layout0_pos + 14..layout0_pos + 16].try_into().unwrap());
    let field2_id =
        u16::from_le_bytes(buf[layout0_pos + 16..layout0_pos + 18].try_into().unwrap());
    let field2_offset =
        u16::from_le_bytes(buf[layout0_pos + 18..layout0_pos + 20].try_into().unwrap());

    use zebin::memoffset::offset_of;
    assert_eq!(field0_id, 0);
    assert_eq!(field0_offset as usize, offset_of!(ArchivedVersionedUser, name));
    assert_eq!(field1_id, 1);
    assert_eq!(field1_offset as usize, offset_of!(ArchivedVersionedUser, id));
    assert_eq!(field2_id, 2);
    assert_eq!(field2_offset as usize, offset_of!(ArchivedVersionedUser, age));
}

#[test]
fn test_safe_access() {
    let user = VersionedUser {
        id: 101,
        age: 30,
        name: "Bob".to_string(),
    };
    let buf = zebin::encode(&user).unwrap();

    let archived = zebin::decode::<VersionedUser>(&buf).expect("Failed to validate archive");
    assert_eq!(archived.schema_id, 0);
    assert_eq!(archived.id, 101);
    assert_eq!(archived.age, 30);
    unsafe {
        assert_eq!(archived.name.as_str(), "Bob");
    }
}
