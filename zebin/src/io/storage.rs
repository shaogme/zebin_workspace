#[cfg(feature = "mmap")]
#[path = "storage/mmap.rs"]
pub mod mmap;

use crate::error::ZebinError;
use crate::utils::cursor::{ChunkSerializer, Cursor, CursorMut, SerializerCursor, SliceCursor};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Unified storage layer: byte-backed read access contract.
pub trait Storage {
    type Cursor<'a>: Cursor<'a>
    where
        Self: 'a;

    fn into_cursor<'a>(self) -> Self::Cursor<'a>
    where
        Self: 'a;
}

/// Unified storage layer: byte-backed write access contract.
pub trait StorageMut {
    type CursorMut<'a>: CursorMut<'a>
    where
        Self: 'a;

    fn into_cursor_mut<'a>(self) -> Self::CursorMut<'a>
    where
        Self: 'a;
}

impl<'a> ChunkSerializer for SliceSerializer<'a> {
    #[inline]
    fn pos(&self) -> usize {
        self.pos()
    }

    #[inline]
    fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError> {
        self.peek_buf_mut(len)
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        self.advance(len);
    }
}

#[cfg(feature = "alloc")]
impl ChunkSerializer for VecSerializer {
    #[inline]
    fn pos(&self) -> usize {
        self.pos()
    }

    #[inline]
    fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError> {
        self.peek_buf_mut(len)
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        self.advance(len);
    }
}

impl<'b> Storage for &'b [u8] {
    type Cursor<'a>
        = SliceCursor<'a>
    where
        Self: 'a;

    #[inline]
    fn into_cursor<'a>(self) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        SliceCursor::new(self, 0)
    }
}

#[cfg(feature = "alloc")]
impl<'b> Storage for &'b Vec<u8> {
    type Cursor<'a>
        = SliceCursor<'a>
    where
        Self: 'a;

    #[inline]
    fn into_cursor<'a>(self) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        SliceCursor::new(self.as_slice(), 0)
    }
}

/// Chunked serializer that writes into a caller-provided buffer slice.
pub struct SliceSerializer<'a> {
    buf: &'a mut [u8],
    written: usize,
    archive_pos: usize,
    write_pos: usize,
}

impl<'a> SliceSerializer<'a> {
    pub fn new(buf: &'a mut [u8], archive_pos: usize) -> Self {
        Self {
            buf,
            written: 0,
            archive_pos,
            write_pos: 0,
        }
    }

    pub fn written(&self) -> usize {
        self.written
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    pub fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError> {
        let pos = self.write_pos;
        let remaining = self.buf.len().saturating_sub(pos);
        let count = remaining.min(len);
        if count == 0 && len > 0 {
            return Ok(&mut []);
        }
        let end = pos + count;
        Ok(&mut self.buf[pos..end])
    }

    #[inline]
    pub fn advance(&mut self, len: usize) {
        let pos = self.write_pos;
        let remaining = self.buf.len().saturating_sub(pos);
        let count = remaining.min(len);
        let next_archive_pos = self.archive_pos.checked_add(count).expect("overflow");
        self.archive_pos = next_archive_pos;
        self.write_pos = pos + count;
        self.written = self.written.max(self.write_pos);
    }
}

#[cfg(feature = "std")]
impl<'a> std::io::Write for SliceSerializer<'a> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let pos = self.write_pos;
        let remaining = self.buf.len().saturating_sub(pos);
        let count = remaining.min(buf.len());
        if count > 0 {
            crate::utils::byteops::copy_exact(&mut self.buf[pos..pos + count], &buf[..count]);
            self.write_pos += count;
            self.archive_pos += count;
            self.written = self.written.max(self.write_pos);
        }
        Ok(count)
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'b, 'c> StorageMut for &'b mut SliceSerializer<'c> {
    type CursorMut<'a>
        = SerializerCursor<'a, SliceSerializer<'c>>
    where
        Self: 'a;

    #[inline]
    fn into_cursor_mut<'a>(self) -> Self::CursorMut<'a>
    where
        Self: 'a,
    {
        SerializerCursor::new(self)
    }
}

#[cfg(feature = "alloc")]
/// Serializer that writes into a dynamically growing vector.
pub struct VecSerializer {
    buf: Vec<u8>,
    archive_pos: usize,
}

#[cfg(feature = "alloc")]
impl VecSerializer {
    pub fn new(archive_pos: usize) -> Self {
        Self {
            buf: Vec::new(),
            archive_pos,
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    #[inline]
    pub fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    pub fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError> {
        let pos = self.archive_pos;
        let end = pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos })?;
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        Ok(&mut self.buf[pos..end])
    }

    #[inline]
    pub fn advance(&mut self, len: usize) {
        self.archive_pos = self.archive_pos.checked_add(len).expect("overflow");
    }
}

#[cfg(feature = "std")]
impl std::io::Write for VecSerializer {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        self.archive_pos += buf.len();
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(feature = "alloc")]
impl<'b> StorageMut for &'b mut VecSerializer {
    type CursorMut<'a>
        = SerializerCursor<'a, VecSerializer>
    where
        Self: 'a;

    #[inline]
    fn into_cursor_mut<'a>(self) -> Self::CursorMut<'a>
    where
        Self: 'a,
    {
        SerializerCursor::new(self)
    }
}

#[cfg(feature = "std")]
pub struct WriteSerializer<W: std::io::Write> {
    writer: W,
    buf: Vec<u8>,
    archive_pos: usize,
}

#[cfg(feature = "std")]
impl<W: std::io::Write> WriteSerializer<W> {
    pub fn new(writer: W, archive_pos: usize) -> Self {
        Self {
            writer,
            buf: Vec::new(),
            archive_pos,
        }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

#[cfg(feature = "std")]
impl<W: std::io::Write> ChunkSerializer for WriteSerializer<W> {
    #[inline]
    fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError> {
        if self.buf.len() < len {
            self.buf.resize(len, 0);
        }
        Ok(&mut self.buf[..len])
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        if len > 0 {
            if self.writer.write_all(&self.buf[..len]).is_ok() {
                self.archive_pos += len;
                self.buf.drain(..len);
            }
        }
    }
}

#[cfg(feature = "std")]
impl<'b, W: std::io::Write> StorageMut for &'b mut WriteSerializer<W> {
    type CursorMut<'a>
        = SerializerCursor<'a, WriteSerializer<W>>
    where
        Self: 'a;

    #[inline]
    fn into_cursor_mut<'a>(self) -> Self::CursorMut<'a>
    where
        Self: 'a,
    {
        SerializerCursor::new(self)
    }
}
