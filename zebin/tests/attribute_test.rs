#[cfg(feature = "alloc")]
use zebin::{ZebinArchive, ZebinEncode};

#[cfg(feature = "alloc")]
#[allow(dead_code)]
#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq, Clone)]
pub struct AttributeTest {
    pub id: u64,

    #[zebin(rename = "name")]
    pub username: String,

    #[zebin(skip)]
    pub ignored: i32,

    #[zebin(skip_serializing)]
    pub also_ignored: String,
}

#[cfg(feature = "alloc")]
#[test]
fn test_rename_and_skip() {
    let user = AttributeTest {
        id: 42,
        username: "Alice".to_string(),
        ignored: 123,
        also_ignored: "Secret".to_string(),
    };

    let buf = zebin::encode(user).unwrap();
    let mut reader_obj = zebin::reader::<AttributeTest, _>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();

    assert_eq!(archived.id, 42);
    // 检查重命名后的字段
    assert_eq!(unsafe { archived.name.as_str() }, "Alice");
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
#[derive(ZebinArchive, ZebinEncode, Clone)]
pub struct TupleTest(u32, #[zebin(skip)] String, u64);

#[cfg(feature = "alloc")]
#[test]
fn test_tuple_skip() {
    let t = TupleTest(1, "ignored".to_string(), 2);
    let buf = zebin::encode(t).unwrap();
    let mut reader_obj = zebin::reader::<TupleTest, _>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();

    assert_eq!(archived.0, 1);
    // 原本 index 为 2 的字段现在应该是 index 为 1 的字段
    assert_eq!(archived.1, 2);
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
#[derive(ZebinArchive, ZebinEncode, Clone)]
pub enum EnumTest {
    Variant1 {
        id: u32,
        #[zebin(skip)]
        secret: String,
    },
    #[zebin(rename = "NewVariant")]
    Variant2 {
        #[zebin(rename = "val")]
        value: u32,
    },
}

#[cfg(feature = "alloc")]
#[test]
fn test_enum_skip_rename() {
    let e1 = EnumTest::Variant1 {
        id: 10,
        secret: "hidden".to_string(),
    };
    let buf1 = zebin::encode(e1).unwrap();
    let mut reader_obj1 = zebin::reader::<EnumTest, _>(&buf1).unwrap();
    let archived1 = reader_obj1.read().unwrap();
    if let Some(v) = archived1.as_variant1() {
        assert_eq!(v.id, 10);
    } else {
        panic!("expected variant1");
    }

    let e2 = EnumTest::Variant2 { value: 20 };
    let buf2 = zebin::encode(e2).unwrap();
    let mut reader_obj2 = zebin::reader::<EnumTest, _>(&buf2).unwrap();
    let archived2 = reader_obj2.read().unwrap();
    if let Some(v) = archived2.as_new_variant() {
        // v.val should be 20 because 'value' was renamed to 'val'
        assert_eq!(v.val, 20);
    } else {
        panic!("expected variant2 (renamed to new_variant)");
    }
}
