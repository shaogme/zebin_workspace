use core::num::NonZeroUsize;

#[cfg(feature = "mmap")]
use crate::io::MmapMut;
use crate::{
    prelude::*,
    utils::{byteops, padding_for_alignment},
};

/// Chunked encoder that writes into a caller-provided buffer slice.
pub struct SliceEncoder<'a> {
    buf: &'a mut [u8],
    written: usize,
    archive_pos: usize,
}

impl<'a> SliceEncoder<'a> {
    pub fn new(buf: &'a mut [u8], archive_pos: usize) -> Self {
        Self {
            buf,
            written: 0,
            archive_pos,
        }
    }

    pub fn written(&self) -> usize {
        self.written
    }

    fn prepare_range(&mut self, len: usize) -> Result<(usize, usize), ZebinError> {
        let remaining_buf = self.buf.len().saturating_sub(self.written);
        let count = remaining_buf.min(len);

        if count == 0 && len > 0 {
            return Ok((0, 0));
        }

        let start = self.written;
        let end = start + count;

        let next_archive_pos =
            self.archive_pos
                .checked_add(count)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;

        self.archive_pos = next_archive_pos;
        self.written = end;
        Ok((start, end))
    }
}

impl ByteSink for SliceEncoder<'_> {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(bytes.len())?;
        let len = end - start;
        if len > 0 {
            self.buf[start..end].copy_from_slice(&bytes[..len]);
        }
        Ok(SinkProgress::from_accepted(bytes.len(), len))
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.archive_pos, alignment);
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(len)?;
        let written = end - start;
        if written > 0 {
            byteops::fill(&mut self.buf[start..end], 0);
        }
        Ok(SinkProgress::from_accepted(len, written))
    }
}

#[cfg(feature = "alloc")]
/// Encoder that writes into a dynamically growing vector.
pub struct VecEncoder {
    buf: alloc::vec::Vec<u8>,
    archive_pos: usize,
}

#[cfg(feature = "alloc")]
impl VecEncoder {
    pub fn new(archive_pos: usize) -> Self {
        Self {
            buf: alloc::vec::Vec::new(),
            archive_pos,
        }
    }

    pub fn into_inner(self) -> alloc::vec::Vec<u8> {
        self.buf
    }
}

#[cfg(feature = "alloc")]
impl ByteSink for VecEncoder {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        let next_pos =
            self.archive_pos
                .checked_add(bytes.len())
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        self.buf.extend_from_slice(bytes);
        self.archive_pos = next_pos;
        Ok(SinkProgress::Complete)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.archive_pos, alignment);
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        let next_pos = self
            .archive_pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow {
                pos: self.archive_pos,
            })?;
        self.buf.resize(self.buf.len() + len, 0);
        self.archive_pos = next_pos;
        Ok(SinkProgress::Complete)
    }
}

#[cfg(feature = "mmap")]
/// Encoder that writes into a pre-sized memory-mapped file.
///
/// The mmap must be sized to fit the entire archive before construction.
/// All writes return `SinkProgress::Complete`; if a write would exceed the
/// map, `ZebinError::BufferTooSmall` is returned.
pub struct MmapEncoder {
    mmap: MmapMut,
    archive_pos: usize,
    written: usize,
}

#[cfg(feature = "mmap")]
impl MmapEncoder {
    pub fn new(mmap: MmapMut, archive_pos: usize) -> Self {
        Self {
            mmap,
            archive_pos,
            written: 0,
        }
    }

    pub fn written(&self) -> usize {
        self.written
    }

    pub fn capacity(&self) -> usize {
        self.mmap.len()
    }

    pub fn into_inner(self) -> MmapMut {
        self.mmap
    }

    pub fn flush(&self) -> Result<(), ZebinError> {
        self.mmap.flush().map_err(ZebinError::from)
    }

    fn prepare_range(&mut self, len: usize) -> Result<(usize, usize), ZebinError> {
        let start = self.written;
        let end = start
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow {
                pos: self.archive_pos,
            })?;
        if end > self.mmap.len() {
            return Err(ZebinError::BufferTooSmall {
                pos: self.archive_pos,
                required: end - self.mmap.len(),
            });
        }
        let next_archive_pos =
            self.archive_pos
                .checked_add(len)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        self.archive_pos = next_archive_pos;
        self.written = end;
        Ok((start, end))
    }
}

#[cfg(feature = "mmap")]
impl ByteSink for MmapEncoder {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(bytes.len())?;
        self.mmap[start..end].copy_from_slice(bytes);
        Ok(SinkProgress::Complete)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.archive_pos, alignment);
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(len)?;
        byteops::fill(&mut self.mmap[start..end], 0);
        Ok(SinkProgress::Complete)
    }
}

/// Helper encoder to handle Schema-Aware object header and footer writing in a reentrant manner.
#[derive(Default, Debug, Clone)]
pub struct SchemaObjectEncoder {
    header_cursor: usize,
    object_start: usize,
    table_start: usize,
    table_offset_cursor: usize,
    object_len_cursor: usize,
}

impl SchemaObjectEncoder {
    pub const fn new() -> Self {
        Self {
            header_cursor: 0,
            object_start: 0,
            table_start: 0,
            table_offset_cursor: 0,
            object_len_cursor: 0,
        }
    }

    #[inline]
    pub fn poll_write_header<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
        stable_schema_key: u32,
        schema_revision: u32,
        field_count: u16,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.header_cursor == 0 {
            self.object_start = sink.pos();
        }
        if self.header_cursor < 12 {
            let mut header = [0u8; 12];
            header[0..4].copy_from_slice(&stable_schema_key.to_le_bytes());
            header[4..8].copy_from_slice(&schema_revision.to_le_bytes());
            header[8..10].copy_from_slice(&field_count.to_le_bytes());
            header[10..12].copy_from_slice(&0u16.to_le_bytes());
            let remaining = 12 - self.header_cursor;
            if sink
                .write(&header[self.header_cursor..])?
                .advance_cursor(&mut self.header_cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }

    #[inline]
    pub fn mark_table_start<S: ByteSink + ?Sized>(&mut self, sink: &mut S) {
        if self.table_start == 0 {
            self.table_start = sink.pos();
        }
    }

    #[inline]
    pub fn poll_write_footer<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.table_offset_cursor < 4 {
            let offset_val = (self.table_start - self.object_start) as u32;
            let offset_bytes = offset_val.to_le_bytes();
            let remaining = 4 - self.table_offset_cursor;
            if sink
                .write(&offset_bytes[self.table_offset_cursor..])?
                .advance_cursor(&mut self.table_offset_cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        if self.object_len_cursor < 4 {
            let total_len = (sink.pos() - self.object_start + 4 - self.object_len_cursor) as u32;
            let len_bytes = total_len.to_le_bytes();
            let remaining = 4 - self.object_len_cursor;
            if sink
                .write(&len_bytes[self.object_len_cursor..])?
                .advance_cursor(&mut self.object_len_cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }
}

/// Helper encoder to serialize a field entry reentrantly.
#[derive(Default, Debug, Clone)]
pub struct FieldEntryEncoder {
    cursor: usize,
}

impl FieldEntryEncoder {
    pub const fn new() -> Self {
        Self { cursor: 0 }
    }

    #[inline]
    pub fn poll_write<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
        field_id: u16,
        encoding: crate::schema::FieldEncoding,
        payload_len: u32,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.cursor < crate::schema::FieldEntry::SIZE {
            let entry = crate::schema::FieldEntry {
                field_id,
                encoding,
                payload_len,
            };
            let bytes = entry.to_bytes();
            let remaining = crate::schema::FieldEntry::SIZE - self.cursor;
            if sink
                .write(&bytes[self.cursor..])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }
}

/// Helper encoder to serialize an enum tag reentrantly.
#[derive(Default, Debug, Clone)]
pub struct TagEncoder {
    bytes: [u8; 4],
    cursor: usize,
}

impl TagEncoder {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 4],
            cursor: 0,
        }
    }

    #[inline]
    pub fn input(&mut self, tag: u32) {
        self.bytes = tag.to_le_bytes();
        self.cursor = 0;
    }

    #[inline]
    pub fn poll_write<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        if self.cursor < 4 {
            let remaining = 4 - self.cursor;
            if sink
                .write(&self.bytes[self.cursor..])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(core::task::Poll::Pending);
            }
        }
        Ok(core::task::Poll::Ready(()))
    }
}
