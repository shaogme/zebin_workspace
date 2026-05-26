#[cfg(feature = "alloc")]
use core::num::NonZeroUsize;
#[cfg(feature = "alloc")]
use std::cell::Cell;
#[cfg(feature = "alloc")]
use zebin::ZebinError;
use zebin::io::SliceEncoder;
#[cfg(feature = "alloc")]
use zebin::prelude::{ByteSink, SinkProgress, ZebinWriter};
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
    let archived = reader::<UserProfile>(&buf).unwrap();
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

    impl ByteSink for LimitedSink<'_> {
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
    let archived = reader::<SimpleUser>(&buf[..written]).unwrap();
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
    let reader_obj = reader::<IterArchive<[u64; 3], u64>>(&buf[..written]).unwrap();
    let archived_iter = reader_obj.root();
    assert_eq!(archived_iter.len(), 3);
    let mut iter = archived_iter.iter();
    assert_eq!(iter.next().unwrap().unwrap(), 10);
    assert_eq!(iter.next().unwrap().unwrap(), 20);
    assert_eq!(iter.next().unwrap().unwrap(), 30);
    assert!(iter.next().is_none());
}
