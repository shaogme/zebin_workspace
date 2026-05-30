#[cfg(feature = "mmap")]
#[path = "storage/mmap.rs"]
pub mod mmap;

use crate::error::ZebinError;
use crate::utils::chunk::BufMut;
use crate::utils::cursor::{Cursor, SliceCursor};
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Storage mode indicating if it supports stream sharding.
pub trait StorageMode {}

/// Static storage mode. Doesn't support advance_shard (or it's a no-op).
pub struct StaticMode;
impl StorageMode for StaticMode {}

/// Stream storage mode. Supports advance_shard to load next chunks.
pub struct StreamMode;
impl StorageMode for StreamMode {}

/// Unified storage layer: byte-backed read access contract.
pub trait Storage {
    type Mode: StorageMode;
    type Cursor<'a>: Cursor<'a>
    where
        Self: 'a;

    #[inline]
    fn advance_sharder(&mut self) -> Result<(), ZebinError> {
        Err(ZebinError::BufferTooSmall {
            pos: 0,
            required: 1,
        })
    }

    fn cursor<'a>(&'a self, pos: usize) -> Self::Cursor<'a>
    where
        Self: 'a;
}

/// Unified storage layer: byte-backed write access contract.
pub trait StorageMut {
    fn pos(&self) -> usize;
    fn peek_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError>;
    fn advance(&mut self, len: usize);
}

impl<S: Storage<Mode = StaticMode> + ?Sized> Storage for &S {
    type Mode = StaticMode;
    type Cursor<'a>
        = S::Cursor<'a>
    where
        Self: 'a;

    #[inline]
    fn advance_sharder(&mut self) -> Result<(), ZebinError> {
        Err(ZebinError::BufferTooSmall {
            pos: 0,
            required: 1,
        })
    }

    #[inline]
    fn cursor<'a>(&'a self, pos: usize) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        (**self).cursor(pos)
    }
}

impl<S: Storage + ?Sized> Storage for &mut S {
    type Mode = S::Mode;
    type Cursor<'a>
        = S::Cursor<'a>
    where
        Self: 'a;

    #[inline]
    fn advance_sharder(&mut self) -> Result<(), ZebinError> {
        (**self).advance_sharder()
    }

    #[inline]
    fn cursor<'a>(&'a self, pos: usize) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        (**self).cursor(pos)
    }
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

impl Storage for [u8] {
    type Mode = StaticMode;
    type Cursor<'a>
        = SliceCursor<'a>
    where
        Self: 'a;

    #[inline]
    fn cursor<'a>(&'a self, pos: usize) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        SliceCursor::new(self, pos)
    }
}

#[cfg(feature = "alloc")]
impl Storage for Vec<u8> {
    type Mode = StaticMode;
    type Cursor<'a>
        = SliceCursor<'a>
    where
        Self: 'a;

    #[inline]
    fn cursor<'a>(&'a self, pos: usize) -> Self::Cursor<'a>
    where
        Self: 'a,
    {
        SliceCursor::new(self.as_slice(), pos)
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
