use zebin::io::SliceSerializer;
use zebin::{ZebinAccess, ZebinDeserialize, ZebinSerialize, deserialize, writer};

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
#[zebin(schema_key = 1)]
pub struct UserProfile {
    #[zebin(id = 0)]
    pub id: u64,
    #[zebin(id = 1)]
    pub username: String,
    #[zebin(id = 2)]
    pub tags: Vec<String>,
}

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
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

#[cfg(feature = "alloc")]
#[test]
fn test_struct_deserialize() {
    let user = UserProfile {
        id: 42,
        username: "Alice".to_string(),
        tags: vec!["rust".to_string(), "zebin".to_string()],
    };
    let buf = zebin::serialize(&user).unwrap();
    let deserialized: UserProfile = zebin::deserialize::<UserProfile, _>(&buf).unwrap();

    assert_eq!(deserialized, user);
}

#[cfg(feature = "alloc")]
#[test]
fn test_enum_deserialize() {
    let ping = Packet::Ping;
    let buf_ping = zebin::serialize(&ping).unwrap();
    let deserialized_ping: Packet = zebin::deserialize::<Packet, _>(&buf_ping).unwrap();
    assert_eq!(deserialized_ping, ping);

    let data = Packet::Data {
        code: 123,
        label: "test".to_string(),
    };
    let buf_data = zebin::serialize(&data).unwrap();
    let deserialized_data: Packet = zebin::deserialize::<Packet, _>(&buf_data).unwrap();
    assert_eq!(deserialized_data, data);
}

#[cfg(feature = "alloc")]
#[test]
fn test_nested_deserialize() {
    #[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
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
    let buf = zebin::serialize(&container).unwrap();
    let deserialized: Container = zebin::deserialize::<Container, _>(&buf).unwrap();
    assert_eq!(deserialized, container);
}

#[cfg(feature = "alloc")]
#[test]
fn test_optional_option_deserialize() {
    #[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
    #[zebin(schema_key = 4)]
    struct OptionalStruct {
        #[zebin(id = 0)]
        pub maybe_string: Option<String>,
        #[zebin(id = 1)]
        pub maybe_u32: Option<u32>,
    }

    // Test with Some
    let obj = OptionalStruct {
        maybe_string: Some("hello".to_string()),
        maybe_u32: Some(100),
    };
    let buf = zebin::serialize(&obj).unwrap();
    let deserialized: OptionalStruct = zebin::deserialize::<OptionalStruct, _>(&buf).unwrap();
    assert_eq!(deserialized, obj);

    // Test with None
    let obj_none = OptionalStruct {
        maybe_string: None,
        maybe_u32: None,
    };
    let buf_none = zebin::serialize(&obj_none).unwrap();
    let deserialized_none: OptionalStruct =
        zebin::deserialize::<OptionalStruct, _>(&buf_none).unwrap();
    assert_eq!(deserialized_none, obj_none);
}

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
pub struct SimpleProfile {
    pub id: u64,
    pub val: u32,
}

#[test]
fn test_struct_deserialize_no_alloc() {
    let profile = SimpleProfile { id: 42, val: 99 };
    let mut buf = [0u8; 128];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<&SimpleProfile, _>(&mut serializer).unwrap();
    writer_obj.write_all(&profile).unwrap();
    let written = serializer.written();
    let deserialized: SimpleProfile = deserialize::<SimpleProfile, _>(&buf[..written]).unwrap();
    assert_eq!(deserialized, profile);
}

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Debug, PartialEq, Clone)]
pub enum SimplePacket {
    Ping,
    Data(u32),
}

#[test]
fn test_enum_deserialize_no_alloc() {
    let ping = SimplePacket::Ping;
    let mut buf = [0u8; 64];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<&SimplePacket, _>(&mut serializer).unwrap();
    writer_obj.write_all(&ping).unwrap();
    let written = serializer.written();
    let deserialized: SimplePacket = deserialize::<SimplePacket, _>(&buf[..written]).unwrap();
    assert_eq!(deserialized, ping);

    let data = SimplePacket::Data(123);
    let mut buf = [0u8; 64];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<&SimplePacket, _>(&mut serializer).unwrap();
    writer_obj.write_all(&data).unwrap();
    let written = serializer.written();
    let deserialized: SimplePacket = deserialize::<SimplePacket, _>(&buf[..written]).unwrap();
    assert_eq!(deserialized, data);
}

#[cfg(feature = "alloc")]
#[test]
fn test_ref_serialize_deserialize() {
    let user = UserProfile {
        id: 42,
        username: "Alice".to_string(),
        tags: vec!["rust".to_string(), "zebin".to_string()],
    };
    let buf = zebin::serialize(&user).unwrap();
    let deserialized: UserProfile = deserialize::<UserProfile, _>(&buf).unwrap();

    assert_eq!(deserialized, user);
}
