use zebin::{ZebinArchive, ZebinEncode};

#[cfg(feature = "alloc")]
#[derive(ZebinArchive, ZebinEncode, Clone)]
pub struct UserProfile {
    pub id: u64,
    pub username: String,
}

#[cfg(feature = "alloc")]
#[test]
fn test_basic_archive() {
    let user = UserProfile {
        id: 42,
        username: "Alice".to_string(),
    };
    let buf = zebin::encode(user).unwrap();
    let archived = zebin::reader::<UserProfile>(&buf).unwrap();
    assert_eq!(archived.id, 42);
    assert_eq!(unsafe { archived.username.as_str() }, "Alice");
}

#[cfg(feature = "alloc")]
#[test]
fn test_encode_into_existing_buffer() {
    let user = UserProfile {
        id: 7,
        username: "Bob".to_string(),
    };

    let expected = zebin::encode(&user).unwrap();
    let mut buf = vec![0xAA, 0xBB, 0xCC];
    zebin::encode_into(user, &mut buf).unwrap();

    assert_eq!(buf, expected);
}

#[cfg(feature = "alloc")]
#[test]
fn test_chunked_writer_resume() {
    let user = UserProfile {
        id: 99,
        username: "Chunky".to_string(),
    };

    let expected = zebin::encode(&user).unwrap();
    let mut writer = zebin::ZebinWriter::<&UserProfile>::new(&user).unwrap();
    let mut actual = Vec::new();
    let mut chunk = [0u8; 5];

    while !writer.is_finished() {
        let written = writer.write(&mut chunk).unwrap();
        if written > 0 {
            actual.extend_from_slice(&chunk[..written]);
        }
    }

    assert_eq!(actual, expected);
}

#[derive(ZebinArchive, ZebinEncode, Clone)]
pub struct SimpleUser {
    pub id: u64,
}

#[test]
fn test_basic_no_alloc() {
    let user = SimpleUser { id: 42 };
    let mut buf = [0u8; 64];
    let mut writer = zebin::encode_chunked(user).unwrap();
    let mut written = 0;
    while !writer.is_finished() {
        let n = writer.write(&mut buf[written..]).unwrap();
        if n == 0 {
            break;
        }
        written += n;
    }
    let archived = zebin::reader::<SimpleUser>(&buf[..written]).unwrap();
    assert_eq!(archived.id, 42);
}

#[test]
fn test_iter_archive_no_alloc() {
    use zebin::archive::IterArchive;
    let arr = [10u64, 20u64, 30u64];
    let wrapped = IterArchive::new(arr);
    let mut buf = [0u8; 128];
    let mut writer = zebin::encode_chunked(wrapped).unwrap();
    let mut written = 0;
    while !writer.is_finished() {
        let n = writer.write(&mut buf[written..]).unwrap();
        if n == 0 {
            break;
        }
        written += n;
    }
    let reader = zebin::reader::<IterArchive<[u64; 3], u64>>(&buf[..written]).unwrap();
    let archived_iter = reader.root();
    assert_eq!(archived_iter.len(), 3);
    let mut iter = archived_iter.iter();
    assert_eq!(iter.next().unwrap().unwrap(), 10);
    assert_eq!(iter.next().unwrap().unwrap(), 20);
    assert_eq!(iter.next().unwrap().unwrap(), 30);
    assert!(iter.next().is_none());
}

