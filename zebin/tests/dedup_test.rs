#![cfg(feature = "alloc")]

use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize, Clone)]
#[zebin(schema_key = 860116326)]
pub struct Child {
    #[zebin(id = 0)]
    pub value: u32,
}

#[derive(ZebinArchive, ZebinSerialize, Clone)]
pub struct Parent {
    pub children: Vec<Child>,
}

#[test]
fn test_vtable_deduplication() {
    let parent = Parent {
        children: vec![Child { value: 1 }, Child { value: 2 }, Child { value: 3 }],
    };
    let buf = zebin::serialize(parent).unwrap();

    assert_eq!(&buf[0..2], b"ZB");
    let archived = zebin::access::<Parent, _>(&buf).unwrap();
    for child_raw in unsafe { archived.children.as_slice() }.iter() {
        assert!(child_raw.value().is_ok());
    }
}
