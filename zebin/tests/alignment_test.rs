#[cfg(feature = "alloc")]
use core::cell::Cell;
use core::num::NonZeroUsize;
use zebin::io::{CursorMut, SinkProgress, SliceSerializer};
#[cfg(feature = "alloc")]
use zebin::prelude::{StorageMut, ZebinWriter};
use zebin::utils::padding_for_alignment;
#[cfg(feature = "alloc")]
use zebin::{ZebinAccess, ZebinDeserialize, ZebinError, ZebinSerialize};
use zebin::{access, writer};

#[cfg(feature = "alloc")]
#[derive(Debug, PartialEq, Eq, ZebinAccess, ZebinDeserialize, ZebinSerialize, Clone)]
struct AlignedContainers {
    values: Vec<u64>,
    fixed: [u64; 3],
}

#[cfg(feature = "alloc")]
#[test]
fn test_aligned_containers_round_trip() {
    let value = AlignedContainers {
        values: vec![11, 22, 33, 44],
        fixed: [55, 66, 77],
    };

    let buf = zebin::serialize(value.clone()).unwrap();
    let archived = access::<AlignedContainers, _>(&buf).unwrap();

    assert_eq!(unsafe { archived.values.as_slice() }, &[11, 22, 33, 44]);
    assert_eq!(archived.fixed, [55, 66, 77]);
}

#[cfg(feature = "alloc")]
#[test]
fn test_aligned_containers_chunked_writer_matches_full_serialize() {
    let value = AlignedContainers {
        values: vec![101, 202, 303],
        fixed: [404, 505, 606],
    };

    let expected = zebin::serialize(&value).unwrap();
    let limit = Cell::new(0usize);

    struct LimitedSink<'a> {
        buf: Vec<u8>,
        limit: &'a Cell<usize>,
        write_pos: usize,
    }

    impl<'a, 'c> CursorMut<'c> for LimitedSink<'a> {
        fn pos(&self) -> usize {
            self.write_pos
        }

        fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
            if bytes.is_empty() {
                return Ok(SinkProgress::Complete);
            }
            let pos = self.write_pos;
            let available = self.limit.get().saturating_sub(pos);
            let count = bytes.len().min(available);
            if count > 0 {
                let end = pos + count;
                if end > self.buf.len() {
                    self.buf.resize(end, 0);
                }
                // we can safely use byteops or slice copy
                self.buf[pos..end].copy_from_slice(&bytes[..count]);
                self.write_pos += count;
            }
            Ok(SinkProgress::from_accepted(bytes.len(), count))
        }

        fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
            let padding = padding_for_alignment(self.pos(), alignment);
            self.skip(padding)
        }

        fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
            if len == 0 {
                return Ok(SinkProgress::Complete);
            }
            let pos = self.write_pos;
            let available = self.limit.get().saturating_sub(pos);
            let count = len.min(available);
            if count > 0 {
                let end = pos + count;
                if end > self.buf.len() {
                    self.buf.resize(end, 0);
                }
                self.buf[pos..end].fill(0);
                self.write_pos += count;
            }
            Ok(SinkProgress::from_accepted(len, count))
        }
    }

    impl<'b, 'a> StorageMut for &'b mut LimitedSink<'a> {
        type CursorMut<'c>
            = &'c mut LimitedSink<'a>
        where
            Self: 'c;

        fn into_cursor_mut<'c>(self) -> Self::CursorMut<'c>
        where
            Self: 'c,
        {
            self
        }
    }

    let mut sink = LimitedSink {
        buf: Vec::new(),
        limit: &limit,
        write_pos: 0,
    };

    let mut writer_obj =
        ZebinWriter::<&AlignedContainers, &mut LimitedSink>::new(&mut sink).unwrap();

    while !writer_obj.is_finished() {
        limit.set(limit.get() + 7);
        let _ = writer_obj.write(&value).unwrap();
    }

    assert_eq!(sink.buf, expected);
    let _ = access::<AlignedContainers, _>(&sink.buf).unwrap();
}

#[cfg(feature = "alloc")]
#[test]
fn test_root_vec_u64_round_trip() {
    let value = vec![7u64, 14, 21, 28];

    let buf = zebin::serialize(value.clone()).unwrap();
    let archived = access::<Vec<u64>, _>(&buf).unwrap();

    assert_eq!(unsafe { archived.as_slice() }, &[7, 14, 21, 28]);
}

#[test]
fn test_root_array_u64_round_trip() {
    let value = [8u64, 16, 32];

    let mut buf = [0u8; 128];
    let mut serializer = SliceSerializer::new(&mut buf, 0);
    let mut writer_obj = writer::<[u64; 3], _>(&mut serializer).unwrap();
    writer_obj.write_all(value).unwrap();
    let written = serializer.written();

    let archived = access::<[u64; 3], _>(&buf[..written]).unwrap();

    assert_eq!(archived, [8, 16, 32]);
}
