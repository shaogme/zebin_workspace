use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize)]
#[zebin(schema_key = 860116326)]
pub struct Child {
    #[zebin(id = 0)]
    pub value: u32,
}

#[derive(ZebinArchive, ZebinSerialize)]
pub struct Parent {
    pub children: Vec<Child>,
}

#[test]
fn test_vtable_deduplication() {
    let parent = Parent {
        children: vec![Child { value: 1 }, Child { value: 2 }, Child { value: 3 }],
    };
    let buf = zebin::encode(&parent).unwrap();

    // Get layout offset from header
    let layout_section_offset = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let num_layouts = u32::from_le_bytes(
        buf[layout_section_offset..layout_section_offset + 4]
            .try_into()
            .unwrap(),
    );

    // There should be exactly 1 layout:
    // Child is evolvable, Parent is stable.
    assert_eq!(
        num_layouts, 1,
        "Expected 1 layout (shared Child), found {}",
        num_layouts
    );
}
