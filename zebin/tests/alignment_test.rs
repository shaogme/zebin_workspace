#[cfg(feature = "alloc")]
use std::cell::Cell;
use zebin::io::SliceSerializer;
#[cfg(feature = "alloc")]
use zebin::prelude::{Buf, BufMut, CursorMut, StorageMut, ZebinWriter};
#[cfg(feature = "alloc")]
use zebin::utils::chunk::{ChunkSource, ChunkSourceMut};
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

    impl<'a> ChunkSource for LimitedSink<'a> {
        fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
            let end = pos
                .checked_add(len)
                .ok_or(ZebinError::ArithmeticOverflow { pos })?;
            if end > self.buf.len() {
                return Err(ZebinError::BufferTooSmall {
                    pos,
                    required: end - self.buf.len(),
                });
            }
            Ok(Buf::new(&self.buf[pos..end]))
        }

        fn total_len(&self) -> usize {
            self.buf.len()
        }
    }

    impl<'a> ChunkSourceMut for LimitedSink<'a> {
        fn get_buf_mut(&mut self, pos: usize, len: usize) -> Result<BufMut<'_>, ZebinError> {
            let available = self.limit.get().saturating_sub(pos);
            let count = len.min(available);
            if count == 0 && len > 0 {
                return Ok(BufMut::new(&mut []));
            }
            let end = pos + count;
            if end > self.buf.len() {
                self.buf.resize(end, 0);
            }
            Ok(BufMut::new(&mut self.buf[pos..end]))
        }
    }

    impl StorageMut for LimitedSink<'_> {
        fn writer(&mut self) -> CursorMut<'_> {
            let pos = self.buf.len();
            CursorMut::new(self, pos)
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
