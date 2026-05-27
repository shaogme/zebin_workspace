#[cfg(feature = "alloc")]
use core::num::NonZeroUsize;
#[cfg(feature = "alloc")]
use std::cell::Cell;
#[cfg(feature = "alloc")]
use zebin::ZebinError;
use zebin::io::SliceEncoder;
#[cfg(feature = "alloc")]
use zebin::prelude::{SinkProgress, StorageMut, ZebinWriter};
use zebin::{ZebinArchive, ZebinEncode, reader, writer};

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
    let mut reader_obj = reader::<UserProfile, _>(&buf).unwrap();
    let archived = reader_obj.read().unwrap();
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

    let _ = &expected;
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
    let limit = Cell::new(0usize);

    struct LimitedSink<'a> {
        buf: Vec<u8>,
        limit: &'a Cell<usize>,
    }

    impl StorageMut for LimitedSink<'_> {
        type Sharder<'b>
            = zebin::io::NoSharder
        where
            Self: 'b;

        fn pos(&self) -> usize {
            self.buf.len()
        }

        fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
            if bytes.is_empty() {
                return Ok(SinkProgress::Complete);
            }
            let available = self.limit.get().saturating_sub(self.buf.len());
            if available == 0 {
                return Ok(SinkProgress::Blocked);
            }
            let write_len = bytes.len().min(available);
            self.buf.extend_from_slice(&bytes[..write_len]);
            Ok(SinkProgress::from_accepted(bytes.len(), write_len))
        }

        fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
            let padding = (alignment.get() - (self.pos() % alignment.get())) % alignment.get();
            self.skip(padding)
        }

        fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
            if len == 0 {
                return Ok(SinkProgress::Complete);
            }
            let available = self.limit.get().saturating_sub(self.buf.len());
            if available == 0 {
                return Ok(SinkProgress::Blocked);
            }
            let skip_len = len.min(available);
            self.buf.resize(self.buf.len() + skip_len, 0);
            Ok(SinkProgress::from_accepted(len, skip_len))
        }

        fn sharder(&mut self) -> Self::Sharder<'_> {
            zebin::io::NoSharder
        }
    }

    let mut sink = LimitedSink {
        buf: Vec::new(),
        limit: &limit,
    };

    let mut writer_obj = ZebinWriter::<&UserProfile, _>::new(&mut sink).unwrap();

    while !writer_obj.is_finished() {
        limit.set(limit.get() + 5);
        let _ = writer_obj.write(&user).unwrap();
    }

    assert_eq!(sink.buf, expected);
}

#[derive(ZebinArchive, ZebinEncode, Clone)]
pub struct SimpleUser {
    pub id: u64,
}

#[test]
fn test_basic_no_alloc() {
    let user = SimpleUser { id: 42 };
    let mut buf = [0u8; 64];
    let mut encoder = SliceEncoder::new(&mut buf, 0);
    let mut writer_obj = writer::<SimpleUser, _>(&mut encoder).unwrap();
    writer_obj.write_all(user).unwrap();
    let written = encoder.written();
    let mut reader_obj = reader::<SimpleUser, _>(&buf[..written]).unwrap();
    let archived = reader_obj.read().unwrap();
    assert_eq!(archived.id, 42);
}

#[test]
fn test_iter_archive_no_alloc() {
    use zebin::archive::IterArchive;
    let arr = [10u64, 20u64, 30u64];
    let wrapped = IterArchive::new(arr);
    let mut buf = [0u8; 128];
    let mut encoder = SliceEncoder::new(&mut buf, 0);
    let mut writer_obj = writer::<IterArchive<[u64; 3], u64>, _>(&mut encoder).unwrap();
    writer_obj.write_all(wrapped).unwrap();
    let written = encoder.written();
    let mut reader_obj = reader::<IterArchive<[u64; 3], u64>, _>(&buf[..written]).unwrap();
    let archived_iter = reader_obj.read().unwrap();
    assert_eq!(archived_iter.len(), 3);
    let mut iter = archived_iter.iter();
    assert_eq!(iter.next().unwrap().unwrap(), 10);
    assert_eq!(iter.next().unwrap().unwrap(), 20);
    assert_eq!(iter.next().unwrap().unwrap(), 30);
    assert!(iter.next().is_none());
}

#[cfg(feature = "alloc")]
#[test]
fn test_consecutive_values() {
    let users = vec![
        UserProfile {
            id: 101,
            username: "Alice".to_string(),
        },
        UserProfile {
            id: 102,
            username: "Bob".to_string(),
        },
        UserProfile {
            id: 103,
            username: "Charlie".to_string(),
        },
    ];

    let mut buf = Vec::new();
    for user in &users {
        let encoded = zebin::encode(user).unwrap();
        buf.extend_from_slice(&encoded);
    }

    let mut reader = zebin::prelude::reader::<UserProfile, _>(&buf).unwrap();

    let u1 = reader.read().unwrap();
    assert_eq!(u1.id, 101);
    assert_eq!(unsafe { u1.username.as_str() }, "Alice");

    let u2 = reader.read().unwrap();
    assert_eq!(u2.id, 102);
    assert_eq!(unsafe { u2.username.as_str() }, "Bob");

    let u3 = reader.read().unwrap();
    assert_eq!(u3.id, 103);
    assert_eq!(unsafe { u3.username.as_str() }, "Charlie");

    assert!(reader.read().is_err());
}

#[cfg(feature = "alloc")]
#[test]
fn test_consecutive_writer_and_reader() {
    let users = vec![
        UserProfile {
            id: 101,
            username: "Alice".to_string(),
        },
        UserProfile {
            id: 102,
            username: "Bob".to_string(),
        },
        UserProfile {
            id: 103,
            username: "Charlie".to_string(),
        },
    ];

    use zebin::io::VecEncoder;
    let mut encoder = VecEncoder::new(0);

    for user in &users {
        let mut writer_obj = writer::<&UserProfile, _>(&mut encoder).unwrap();
        writer_obj.write_all(user).unwrap();
    }

    let buf = encoder.into_inner();
    let mut reader = zebin::prelude::reader::<UserProfile, _>(&buf).unwrap();

    let u1 = reader.read().unwrap();
    assert_eq!(u1.id, 101);
    assert_eq!(unsafe { u1.username.as_str() }, "Alice");

    let u2 = reader.read().unwrap();
    assert_eq!(u2.id, 102);
    assert_eq!(unsafe { u2.username.as_str() }, "Bob");

    let u3 = reader.read().unwrap();
    assert_eq!(u3.id, 103);
    assert_eq!(unsafe { u3.username.as_str() }, "Charlie");

    assert!(reader.read().is_err());
}

#[cfg(feature = "alloc")]
struct ShardedStorage {
    shards: Vec<Vec<u8>>,
    current_index: usize,
}

#[cfg(feature = "alloc")]
impl<'a> zebin::io::Sharder for &'a mut ShardedStorage {
    fn advance(&mut self) -> Result<(), ZebinError> {
        if self.current_index + 1 < self.shards.len() {
            self.current_index += 1;
            Ok(())
        } else {
            Err(ZebinError::BufferTooSmall {
                pos: 0,
                required: 1,
            })
        }
    }
}

#[cfg(feature = "alloc")]
impl zebin::io::Storage for ShardedStorage {
    type Mode = zebin::io::StreamMode;
    type Sharder<'a>
        = &'a mut ShardedStorage
    where
        Self: 'a;

    fn as_slice(&self) -> &[u8] {
        if self.current_index < self.shards.len() {
            &self.shards[self.current_index]
        } else {
            &[]
        }
    }

    fn sharder(&mut self) -> Self::Sharder<'_> {
        self
    }
}

#[cfg(feature = "alloc")]
#[test]
fn test_sharded_storage_stream() {
    // Compile-time checks for Storage bounds
    fn assert_storage<S: zebin::io::Storage + ?Sized>() {}
    assert_storage::<[u8]>();
    assert_storage::<&[u8]>();
    assert_storage::<ShardedStorage>();
    // Uncommenting the line below must fail to compile:
    // assert_storage::<&ShardedStorage>();

    let u1 = UserProfile {
        id: 1,
        username: "Alice".to_string(),
    };
    let u2 = UserProfile {
        id: 2,
        username: "Bob".to_string(),
    };

    let shard1 = zebin::encode(&u1).unwrap();
    let shard2 = zebin::encode(&u2).unwrap();

    let storage = ShardedStorage {
        shards: vec![shard1, shard2],
        current_index: 0,
    };

    let mut reader = reader::<UserProfile, _>(storage).unwrap();

    let r1 = reader.read().unwrap();
    assert_eq!(r1.id, 1);
    assert_eq!(unsafe { r1.username.as_str() }, "Alice");

    let r2 = reader.read().unwrap();
    assert_eq!(r2.id, 2);
    assert_eq!(unsafe { r2.username.as_str() }, "Bob");

    assert!(reader.read().is_err());
}
