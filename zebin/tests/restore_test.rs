use zebin::{ZebinArchive, ZebinSerialize};

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
#[zebin(schema_key = 1)]
pub struct UserProfile {
    #[zebin(id = 0)]
    pub id: u64,
    #[zebin(id = 1)]
    pub username: String,
    #[zebin(id = 2)]
    pub tags: Vec<String>,
}

#[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
pub enum Packet {
    Ping,
    #[zebin(schema_key = 2)]
    Data {
        #[zebin(id = 0)]
        code: u32,
        #[zebin(id = 1)]
        label: String,
    },
}

#[test]
fn test_struct_restore() {
    let user = UserProfile {
        id: 42,
        username: "Alice".to_string(),
        tags: vec!["rust".to_string(), "zebin".to_string()],
    };
    let buf = zebin::encode(&user).unwrap();
    let restored: UserProfile = zebin::decode::<UserProfile>(&buf).unwrap();

    assert_eq!(restored, user);
}

#[test]
fn test_enum_restore() {
    let ping = Packet::Ping;
    let buf_ping = zebin::encode(&ping).unwrap();
    let restored_ping: Packet = zebin::decode::<Packet>(&buf_ping).unwrap();
    assert_eq!(restored_ping, ping);

    let data = Packet::Data {
        code: 123,
        label: "test".to_string(),
    };
    let buf_data = zebin::encode(&data).unwrap();
    let restored_data: Packet = zebin::decode::<Packet>(&buf_data).unwrap();
    assert_eq!(restored_data, data);
}

#[test]
fn test_nested_restore() {
    #[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
    #[zebin(schema_key = 3)]
    struct Container {
        #[zebin(id = 0)]
        inner: UserProfile,
    }

    let container = Container {
        inner: UserProfile {
            id: 1,
            username: "Bob".to_string(),
            tags: vec![],
        },
    };
    let buf = zebin::encode(&container).unwrap();
    let restored: Container = zebin::decode::<Container>(&buf).unwrap();
    assert_eq!(restored, container);
}

#[test]
fn test_optional_option_restore() {
    #[derive(ZebinArchive, ZebinSerialize, Debug, PartialEq)]
    #[zebin(schema_key = 4)]
    struct OptionalStruct {
        #[zebin(id = 0, optional)]
        pub maybe_string: Option<String>,
        #[zebin(id = 1, optional)]
        pub maybe_u32: Option<u32>,
    }

    // Test with Some
    let obj = OptionalStruct {
        maybe_string: Some("hello".to_string()),
        maybe_u32: Some(100),
    };
    let buf = zebin::encode(&obj).unwrap();
    let restored: OptionalStruct = zebin::decode::<OptionalStruct>(&buf).unwrap();
    assert_eq!(restored, obj);

    // Test with None
    let obj_none = OptionalStruct {
        maybe_string: None,
        maybe_u32: None,
    };
    let buf_none = zebin::encode(&obj_none).unwrap();
    let restored_none: OptionalStruct = zebin::decode::<OptionalStruct>(&buf_none).unwrap();
    assert_eq!(restored_none, obj_none);
}
