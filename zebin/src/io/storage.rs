#[cfg(feature = "mmap")]
#[path = "storage/mmap.rs"]
pub mod mmap;

use crate::error::ZebinError;
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

/// Shard controller trait.
pub trait Sharder {
    fn advance(&mut self) -> Result<(), ZebinError>;
}

/// No-op sharder for static storage.
pub struct NoSharder;
impl Sharder for NoSharder {
    #[inline]
    fn advance(&mut self) -> Result<(), ZebinError> {
        Err(ZebinError::BufferTooSmall {
            pos: 0,
            required: 1,
        })
    }
}

use crate::utils::chunk::{Buf, BufMut, ChunkSource, ChunkSourceMut};

/// Unified storage layer: byte-backed read access contract.
pub trait Storage: ChunkSource {
    type Mode: StorageMode;
    type Sharder<'a>: Sharder
    where
        Self: 'a;

    fn sharder(&mut self) -> Self::Sharder<'_>;
}

/// Unified storage layer: byte-backed write access contract.
pub trait StorageMut: ChunkSourceMut {}

impl<S: Storage<Mode = StaticMode> + ?Sized> Storage for &S {
    type Mode = StaticMode;
    type Sharder<'a>
        = NoSharder
    where
        Self: 'a;

    #[inline]
    fn sharder(&mut self) -> Self::Sharder<'_> {
        NoSharder
    }
}

impl<S: Storage + ?Sized> Storage for &mut S {
    type Mode = S::Mode;
    type Sharder<'a>
        = S::Sharder<'a>
    where
        Self: 'a;

    #[inline]
    fn sharder(&mut self) -> Self::Sharder<'_> {
        (**self).sharder()
    }
}

impl<S: StorageMut + ?Sized> StorageMut for &mut S {}

impl ChunkSource for [u8] {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        let end = pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos })?;
        if end > self.len() {
            return Err(ZebinError::BufferTooSmall {
                pos,
                required: end - self.len(),
            });
        }
        Ok(Buf::new(&self[pos..end]))
    }

    #[inline]
    fn is_eof(&self, pos: usize) -> bool {
        pos >= self.len()
    }
}

impl Storage for [u8] {
    type Mode = StaticMode;
    type Sharder<'a>
        = NoSharder
    where
        Self: 'a;

    #[inline]
    fn sharder(&mut self) -> Self::Sharder<'_> {
        NoSharder
    }
}

#[cfg(feature = "alloc")]
impl ChunkSource for Vec<u8> {
    #[inline]
    fn get_buf(&self, pos: usize, len: usize) -> Result<Buf<'_>, ZebinError> {
        let end = pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos })?;
        if end > self.len() {
            return Err(ZebinError::BufferTooSmall {
                pos,
                required: end - self.len(),
            });
        }
        Ok(Buf::new(&self[pos..end]))
    }

    #[inline]
    fn is_eof(&self, pos: usize) -> bool {
        pos >= self.len()
    }
}

#[cfg(feature = "alloc")]
impl Storage for Vec<u8> {
    type Mode = StaticMode;
    type Sharder<'a>
        = NoSharder
    where
        Self: 'a;

    #[inline]
    fn sharder(&mut self) -> Self::Sharder<'_> {
        NoSharder
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

impl<'a> ChunkSource for SliceSerializer<'a> {
    #[inline]
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

    #[inline]
    fn is_eof(&self, pos: usize) -> bool {
        pos >= self.buf.len()
    }
}

impl<'a> ChunkSourceMut for SliceSerializer<'a> {
    #[inline]
    fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    fn get_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError> {
        let pos = self.write_pos;
        let remaining = self.buf.len().saturating_sub(pos);
        let count = remaining.min(len);
        if count == 0 && len > 0 {
            return Ok(BufMut::new(&mut []));
        }
        let end = pos + count;

        let next_archive_pos =
            self.archive_pos
                .checked_add(count)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;

        self.archive_pos = next_archive_pos;
        self.write_pos = end;
        self.written = self.written.max(end);
        Ok(BufMut::new(&mut self.buf[pos..end]))
    }
}

impl StorageMut for SliceSerializer<'_> {}

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
impl ChunkSource for VecSerializer {
    #[inline]
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

    #[inline]
    fn is_eof(&self, pos: usize) -> bool {
        pos >= self.buf.len()
    }
}

#[cfg(feature = "alloc")]
impl ChunkSourceMut for VecSerializer {
    #[inline]
    fn pos(&self) -> usize {
        self.archive_pos
    }

    #[inline]
    fn get_buf_mut(&mut self, len: usize) -> Result<BufMut<'_>, ZebinError> {
        let pos = self.archive_pos;
        let end = pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos })?;
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
        self.archive_pos = end;
        Ok(BufMut::new(&mut self.buf[pos..end]))
    }
}

#[cfg(feature = "alloc")]
impl StorageMut for VecSerializer {}
