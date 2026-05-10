use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
#[zebin(schema_key = 0x100)]
pub struct Version1 {
    #[zebin(id = 1)]
    pub id: u32,
    #[zebin(id = 2)]
    pub name: String,
}

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
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

    let reader = zebin::decode::<Version2>(&buf).unwrap();
    let archived = reader.root();

    // Automatically resolve the layout from the object's self-describing metadata
    let layout = reader.get_layout(archived).unwrap();

    assert_eq!(unsafe { archived.id(&layout) }.unwrap(), &42);
    assert_eq!(unsafe { archived.name(&layout).unwrap().as_str() }, "Alice");

    // Missing field with optional
    assert!(unsafe { archived.email(&layout) }.unwrap().is_none());

    // Missing field with default
    assert_eq!(unsafe { archived.age(&layout) }.unwrap(), &0);

    // Missing field with custom default
    assert_eq!(unsafe { archived.score(&layout) }.unwrap(), &100);
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
    let reader = zebin::decode::<Version2>(&buf).unwrap();
    let archived = reader.root();
    let layout = reader.get_layout(archived).unwrap();

    assert_eq!(unsafe { archived.id(&layout) }.unwrap(), &1);
    assert_eq!(unsafe { archived.name(&layout).unwrap().as_str() }, "Bob");
    assert_eq!(
        unsafe { archived.email(&layout).unwrap().unwrap().as_str() },
        "bob@example.com"
    );
    assert_eq!(unsafe { archived.age(&layout) }.unwrap(), &30);
    assert_eq!(unsafe { archived.score(&layout) }.unwrap(), &95);
}

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
pub enum MessageV1 {
    #[zebin(schema_key = 0x201)]
    Login {
        #[zebin(id = 1)]
        user: String,
    },
}

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
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

    let reader = zebin::decode::<MessageV2>(&buf).unwrap();
    let archived = reader.root();

    // Check if it's Login variant
    let login = unsafe { archived.as_login() }.expect("Should be Login variant");

    let layout = reader.get_layout(login).unwrap();

    assert_eq!(unsafe { login.user(&layout).unwrap().as_str() }, "Alice");
    assert!(unsafe { login.device(&layout) }.unwrap().is_none());
}
