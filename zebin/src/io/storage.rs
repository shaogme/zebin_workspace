#[cfg(feature = "mmap")]
#[path = "storage/mmap.rs"]
pub mod mmap;

use crate::error::ZebinError;
use crate::utils::cursor::{Cursor, SliceCursor};
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

use crate::traits_impl::SinkProgress;
use crate::utils::byteops;
use crate::utils::cursor::CursorMut;
use crate::utils::padding_for_alignment;
use core::num::NonZeroUsize;

pub struct SliceSerializerCursor<'a, 'b> {
    serializer: &'a mut SliceSerializer<'b>,
}

impl<'a, 'b> SliceSerializerCursor<'a, 'b> {
    #[inline]
    pub fn new(serializer: &'a mut SliceSerializer<'b>) -> Self {
        Self { serializer }
    }

    #[inline]
    pub fn into_inner(self) -> &'a mut SliceSerializer<'b> {
        self.serializer
    }
}

impl<'a, 'b, 'c> CursorMut<'c> for SliceSerializerCursor<'a, 'b>
where
    'a: 'c,
{
    #[inline]
    fn pos(&self) -> usize {
        self.serializer.pos()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let buf = self.serializer.peek_buf_mut(bytes.len())?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::copy_exact(buf, &bytes[..accepted]);
            self.serializer.advance(accepted);
        }
        Ok(SinkProgress::from_accepted(bytes.len(), accepted))
    }

    #[inline]
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.pos(), alignment);
        self.skip(padding)
    }

    #[inline]
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let buf = self.serializer.peek_buf_mut(len)?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::fill(buf, 0);
            self.serializer.advance(accepted);
        }
        Ok(SinkProgress::from_accepted(len, accepted))
    }
}

#[cfg(feature = "alloc")]
pub struct VecSerializerCursor<'a> {
    serializer: &'a mut VecSerializer,
}

#[cfg(feature = "alloc")]
impl<'a> VecSerializerCursor<'a> {
    #[inline]
    pub fn new(serializer: &'a mut VecSerializer) -> Self {
        Self { serializer }
    }

    #[inline]
    pub fn into_inner(self) -> &'a mut VecSerializer {
        self.serializer
    }
}

#[cfg(feature = "alloc")]
impl<'a, 'b> CursorMut<'b> for VecSerializerCursor<'a>
where
    'a: 'b,
{
    #[inline]
    fn pos(&self) -> usize {
        self.serializer.pos()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let buf = self.serializer.peek_buf_mut(bytes.len())?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::copy_exact(buf, &bytes[..accepted]);
            self.serializer.advance(accepted);
        }
        Ok(SinkProgress::from_accepted(bytes.len(), accepted))
    }

    #[inline]
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.pos(), alignment);
        self.skip(padding)
    }

    #[inline]
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let buf = self.serializer.peek_buf_mut(len)?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::fill(buf, 0);
            self.serializer.advance(accepted);
        }
        Ok(SinkProgress::from_accepted(len, accepted))
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

impl<'b, 'c> StorageMut for &'b mut SliceSerializer<'c> {
    type CursorMut<'a>
        = SliceSerializerCursor<'a, 'c>
    where
        Self: 'a;

    #[inline]
    fn into_cursor_mut<'a>(self) -> Self::CursorMut<'a>
    where
        Self: 'a,
    {
        SliceSerializerCursor::new(self)
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

#[cfg(feature = "alloc")]
impl<'b> StorageMut for &'b mut VecSerializer {
    type CursorMut<'a>
        = VecSerializerCursor<'a>
    where
        Self: 'a;

    #[inline]
    fn into_cursor_mut<'a>(self) -> Self::CursorMut<'a>
    where
        Self: 'a,
    {
        VecSerializerCursor::new(self)
    }
}
