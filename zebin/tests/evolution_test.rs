use zebin::{ZebinArchive, ZebinEncode};

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq)]
#[zebin(schema_key = 0x100)]
pub struct Version1 {
    #[zebin(id = 1)]
    pub id: u32,
    #[zebin(id = 2)]
    pub name: String,
}

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq)]
#[zebin(schema_key = 0x100)]
pub struct Version2 {
    #[zebin(id = 1)]
    pub id: u32,
    #[zebin(id = 2)]
    pub name: String,
    #[zebin(id = 3, optional)]
    pub email: String,
    #[zebin(id = 4, default)]
    pub age: u32,
    #[zebin(id = 5, default_value = "custom_default()")]
    pub score: u32,
}

fn custom_default() -> &'static u32 {
    static VAL: u32 = 100;
    &VAL
}

#[test]
fn test_evolution_optional_and_default() {
    let v1 = Version1 {
        id: 42,
        name: "Alice".to_string(),
    };
    let buf = zebin::encode(&v1).unwrap();

    let reader = zebin::reader::<Version2>(&buf).unwrap();

    // Directly access fields on the reader (it derefs to the root view)
    assert_eq!(reader.id().unwrap(), &42);
    assert_eq!(unsafe { reader.name().unwrap().as_str() }, "Alice");

    // Missing field with optional
    assert!(reader.email().unwrap().is_none());

    // Missing field with default
    assert_eq!(reader.age().unwrap(), &0);

    // Missing field with custom default
    assert_eq!(reader.score().unwrap(), &100);
}

#[test]
fn test_version2_with_all_fields() {
    let v2 = Version2 {
        id: 1,
        name: "Bob".to_string(),
        email: "bob@example.com".to_string(),
        age: 30,
        score: 95,
    };
    let buf = zebin::encode(&v2).unwrap();
    let reader = zebin::reader::<Version2>(&buf).unwrap();

    assert_eq!(reader.id().unwrap(), &1);
    assert_eq!(unsafe { reader.name().unwrap().as_str() }, "Bob");
    assert_eq!(
        unsafe { reader.email().unwrap().unwrap().as_str() },
        "bob@example.com"
    );
    assert_eq!(reader.age().unwrap(), &30);
    assert_eq!(reader.score().unwrap(), &95);
}

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq)]
pub enum MessageV1 {
    #[zebin(schema_key = 0x201)]
    Login {
        #[zebin(id = 1)]
        user: String,
    },
}

#[derive(ZebinArchive, ZebinEncode, Debug, PartialEq)]
pub enum MessageV2 {
    #[zebin(schema_key = 0x201)]
    Login {
        #[zebin(id = 1)]
        user: String,
        #[zebin(id = 2, optional)]
        device: String,
    },
}

#[test]
fn test_enum_evolution() {
    let m1 = MessageV1::Login {
        user: "Alice".to_string(),
    };
    let buf = zebin::encode(&m1).unwrap();

    let reader = zebin::reader::<MessageV2>(&buf).unwrap();

    // Directly access variants on the reader.
    // The variant accessor on View (for enums) returns a nested View for the variant record.
    let login = reader.as_login().expect("Should be Login variant");

    assert_eq!(unsafe { login.user().unwrap().as_str() }, "Alice");
    assert!(login.device().unwrap().is_none());
}
