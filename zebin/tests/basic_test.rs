#[cfg(feature = "alloc")]
use core::cell::Cell;
#[cfg(feature = "alloc")]
use zebin::ZebinError;
use zebin::io::SliceSerializer;
#[cfg(feature = "alloc")]
use zebin::prelude::{Storage, StorageMut, ValidationConfig, ZebinReader, ZebinWriter};
use zebin::{ZebinAccess, ZebinDeserialize, ZebinSerialize, writer};

#[cfg(feature = "alloc")]
#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
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
    let buf = zebin::serialize(user).unwrap();
    let archived = zebin::access::<UserProfile, _>(&buf).unwrap();
    assert_eq!(archived.id, 42);
    assert_eq!(unsafe { archived.username.as_str() }, "Alice");
}

#[cfg(feature = "alloc")]
#[test]
fn test_serialize_into_existing_buffer() {
    let user = UserProfile {
        id: 7,
        username: "Bob".to_string(),
    };

    let expected = zebin::serialize(&user).unwrap();
    let mut buf = vec![0xAA, 0xBB, 0xCC];
    zebin::serialize_into(user, &mut buf).unwrap();

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

    let expected = zebin::serialize(&user).unwrap();
    let limit = Cell::new(0usize);

    struct LimitedSink<'a> {
        buf: Vec<u8>,
        limit: &'a Cell<usize>,
        write_pos: usize,
    }

    impl<'a> StorageMut for LimitedSink<'a> {
        fn pos(&self) -> usize {
            self.write_pos
        }

        fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError> {
            let pos = self.write_pos;
            let available = self.limit.get().saturating_sub(pos);
            let count = len.min(available);
            if count == 0 && len > 0 {
                return Ok(&mut []);
            }
            let end = pos + count;
            if end > self.buf.len() {
                self.buf.resize(end, 0);
            }
            Ok(&mut self.buf[pos..end])
        }

        fn advance(&mut self, len: usize) {
            let pos = self.write_pos;
            let available = self.limit.get().saturating_sub(pos);
            let count = len.min(available);
            self.write_pos += count;
        }
    }

    let mut sink = LimitedSink {
        buf: Vec::new(),
        limit: &limit,
        write_pos: 0,
    };

    let mut writer_obj = ZebinWriter::<&UserProfile, _>::new(&mut sink).unwrap();

    while !writer_obj.is_finished() {
        limit.set(limit.get() + 5);
        let _ = writer_obj.write(&user).unwrap();
    }

    assert_eq!(sink.buf, expected);
}

#[derive(ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
pub struct SimpleUser {
    pub id: u64,
}

#[test]
fn test_basic_no_alloc() {
    let user = SimpleUser { id: 42 };
    let mut buf = [0u8; 64];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<SimpleUser, _>(&mut serializer).unwrap();
    writer_obj.write_all(user).unwrap();
    let written = serializer.written();
    let archived = zebin::access::<SimpleUser, _>(&buf[..written]).unwrap();
    assert_eq!(archived.id, 42);
}

#[test]
fn test_iter_archive_no_alloc() {
    use zebin::archive::IterArchive;
    let arr = [10u64, 20u64, 30u64];
    let wrapped = IterArchive::new(arr);
    let mut buf = [0u8; 128];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<IterArchive<[u64; 3], u64>, _>(&mut serializer).unwrap();
    writer_obj.write_all(wrapped).unwrap();
    let written = serializer.written();
    let archived_iter = zebin::access::<IterArchive<[u64; 3], u64>, _>(&buf[..written]).unwrap();
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
        let serialized = zebin::serialize(user).unwrap();
        buf.extend_from_slice(&serialized);
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

    use zebin::io::VecSerializer;
    let mut serializer = VecSerializer::new(0);

    for user in &users {
        let mut writer_obj = writer::<&UserProfile, _>(&mut serializer).unwrap();
        writer_obj.write_all(user).unwrap();
    }

    let buf = serializer.into_inner();
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
struct ShardedStorageCursor<'a> {
    storage: &'a ShardedStorage,
    pos: usize,
}

#[cfg(feature = "alloc")]
impl<'a> ShardedStorageCursor<'a> {
    fn new(storage: &'a ShardedStorage) -> Self {
        Self { storage, pos: 0 }
    }
}

#[cfg(feature = "alloc")]
impl<'a, 'b> zebin::io::Cursor<'b> for ShardedStorageCursor<'a>
where
    'a: 'b,
{
    #[inline]
    fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), zebin::prelude::AccessError>
    where
        C: zebin::validation::ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        let shard = &self.storage.shards[self.storage.current_index];
        if end > shard.len() {
            return Err(context.validation_error("Pointer out of bounds", self.pos));
        }
        self.pos = end;
        Ok(())
    }

    #[inline]
    fn peek_buf<C>(
        &self,
        len: usize,
        context: &mut C,
    ) -> Result<&'b [u8], zebin::prelude::AccessError>
    where
        C: zebin::validation::ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        let shard = &self.storage.shards[self.storage.current_index];
        if end > shard.len() {
            return Err(context.validation_error("Pointer out of bounds", self.pos));
        }
        Ok(&shard[self.pos..end])
    }

    #[inline]
    fn is_eof(&self) -> bool {
        let shard = &self.storage.shards[self.storage.current_index];
        self.pos >= shard.len()
    }
}

#[cfg(feature = "alloc")]
impl<'b> zebin::io::Storage for &'b ShardedStorage {
    type Cursor<'a>
        = ShardedStorageCursor<'a>
    where
        Self: 'a;

    fn into_cursor<'a>(self) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        ShardedStorageCursor::new(self)
    }
}

#[cfg(feature = "alloc")]
impl ShardedStorage {
    fn advance_sharder(&mut self) -> Result<(), ZebinError> {
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
#[test]
fn test_sharded_storage_stream() {
    // Compile-time checks for Storage bounds
    fn assert_storage<S: zebin::io::Storage + ?Sized>() {}
    assert_storage::<&[u8]>();
    assert_storage::<&ShardedStorage>();

    let u1 = UserProfile {
        id: 1,
        username: "Alice".to_string(),
    };
    let u2 = UserProfile {
        id: 2,
        username: "Bob".to_string(),
    };

    let shard1 = zebin::serialize(&u1).unwrap();
    let shard2 = zebin::serialize(&u2).unwrap();

    let mut storage = ShardedStorage {
        shards: vec![shard1, shard2],
        current_index: 0,
    };

    let cursor = (&storage).into_cursor();
    let mut reader =
        ZebinReader::<UserProfile, _>::new(cursor, ValidationConfig::default()).unwrap();

    let r1 = reader.read().unwrap();
    assert_eq!(r1.id, 1);
    assert_eq!(unsafe { r1.username.as_str() }, "Alice");

    storage.advance_sharder().unwrap();
    let cursor = (&storage).into_cursor();
    let mut reader =
        ZebinReader::<UserProfile, _>::new(cursor, ValidationConfig::default()).unwrap();

    let r2 = reader.read().unwrap();
    assert_eq!(r2.id, 2);
    assert_eq!(unsafe { r2.username.as_str() }, "Bob");

    assert!(reader.read().is_err());
}
