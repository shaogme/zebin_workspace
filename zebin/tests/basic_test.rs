use zebin::{ZebinArchive, ZebinArchiveBuilder};

#[derive(ZebinArchive, ZebinArchiveBuilder)]
pub struct UserProfile {
    pub id: u64,
    pub username: String,
}

#[test]
fn test_basic_archive() {
    let user = UserProfile {
        id: 42,
        username: "Alice".to_string(),
    };
    let buf = zebin::encode(&user).unwrap();
    let archived = zebin::decode::<UserProfile>(&buf).unwrap();
    assert_eq!(archived.id, 42);
    assert_eq!(unsafe { archived.username.as_str() }, "Alice");
}

#[test]
fn test_encode_into_existing_buffer() {
    let user = UserProfile {
        id: 7,
        username: "Bob".to_string(),
    };

    let expected = zebin::encode(&user).unwrap();
    let mut buf = vec![0xAA, 0xBB, 0xCC];
    zebin::encode_into(&user, &mut buf).unwrap();

    assert_eq!(buf, expected);
}

#[test]
fn test_chunked_writer_resume() {
    let user = UserProfile {
        id: 99,
        username: "Chunky".to_string(),
    };

    let expected = zebin::encode(&user).unwrap();
    let mut writer = zebin::ArchiveWriter::new(&user).unwrap();
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
