#[cfg(feature = "mmap")]
#[path = "storage/mmap.rs"]
pub mod mmap;

use crate::error::ZebinError;
use crate::utils::chunk::BufMut;
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
    fn pos(&self) -> usize;
    fn peek_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError>;
    fn advance(&mut self, len: usize);
}

impl<S: StorageMut + ?Sized> StorageMut for &mut S {
    #[inline]
    fn pos(&self) -> usize {
        (**self).pos()
    }

    #[inline]
    fn peek_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError> {
        (**self).peek_buf_mut(len)
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        (**self).advance(len);
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
}

impl<'a> StorageMut for SliceSerializer<'a> {
    #[inline]
    fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    fn peek_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError> {
        let pos = self.write_pos;
        let remaining = self.buf.len().saturating_sub(pos);
        let count = remaining.min(len);
        if count == 0 && len > 0 {
            return Ok(BufMut::new(&mut []));
        }
        let end = pos + count;
        Ok(BufMut::new(&mut self.buf[pos..end]))
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        let pos = self.write_pos;
        let remaining = self.buf.len().saturating_sub(pos);
        let count = remaining.min(len);
        let next_archive_pos = self.archive_pos.checked_add(count).expect("overflow");
        self.archive_pos = next_archive_pos;
        self.write_pos = pos + count;
        self.written = self.written.max(self.write_pos);
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
}

#[cfg(feature = "alloc")]
impl StorageMut for VecSerializer {
    #[inline]
    fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    fn peek_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError> {
        let pos = self.archive_pos;
        let end = pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos })?;
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        Ok(BufMut::new(&mut self.buf[pos..end]))
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        self.archive_pos = self.archive_pos.checked_add(len).expect("overflow");
    }
}
