use zebin::{ZebinArchive, ZebinEncode};

#[derive(ZebinArchive, ZebinEncode)]
#[zebin(schema_key = 860116326)]
pub struct Child {
    #[zebin(id = 0)]
    pub value: u32,
}

#[derive(ZebinArchive, ZebinEncode)]
pub struct Parent {
    pub children: Vec<Child>,
}

#[test]
fn test_vtable_deduplication() {
    let parent = Parent {
        children: vec![Child { value: 1 }, Child { value: 2 }, Child { value: 3 }],
    };
    let buf = zebin::encode(&parent).unwrap();

    assert_eq!(&buf[0..2], b"ZB");

    let reader = zebin::reader::<Parent>(&buf).unwrap();
    let archived = reader.root();
    for child_raw in unsafe { archived.children.as_slice() }.iter() {
        assert!(child_raw.value().is_ok());
    }
}
