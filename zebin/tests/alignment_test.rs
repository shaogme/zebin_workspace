#[cfg(feature = "alloc")]
use core::num::NonZeroUsize;
#[cfg(feature = "alloc")]
use std::cell::Cell;
use zebin::io::SliceSerializer;
#[cfg(feature = "alloc")]
use zebin::prelude::{CursorMut, SinkProgress, StorageMut, ZebinWriter};
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
    }

    impl<'a> zebin::utils::chunk::ChunkSource for LimitedSink<'a> {
        fn chunk_count(&self) -> usize {
            1
        }

        fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
            if idx == 0 {
                Some(self.buf.as_slice())
            } else {
                None
            }
        }
    }

    impl<'a> zebin::utils::chunk::ChunkSourceMut for LimitedSink<'a> {
        fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
            if idx == 0 {
                Some(self.buf.as_mut_slice())
            } else {
                None
            }
        }
    }

    impl StorageMut for LimitedSink<'_> {
        type Writer<'b>
            = &'b mut Self
        where
            Self: 'b;

        fn writer(&mut self) -> Self::Writer<'_> {
            self
        }
    }

    impl CursorMut for LimitedSink<'_> {
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

    let mut writer_obj = ZebinWriter::<&AlignedContainers, _>::new(&mut sink).unwrap();

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

    let archived = access::<[u64; 3], _>(&&buf[..written]).unwrap();

    assert_eq!(archived, [8, 16, 32]);
}
