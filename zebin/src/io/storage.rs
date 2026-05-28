#[cfg(feature = "mmap")]
#[path = "storage/mmap.rs"]
pub mod mmap;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::{
    error::ZebinError,
    traits_impl::SinkProgress,
    utils::{byteops, padding_for_alignment},
};

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

use crate::utils::chunk::{ChunkSource, ChunkSourceMut};

/// Unified storage layer: byte-backed read access contract.
pub trait Storage: ChunkSource {
    type Mode: StorageMode;
    type Sharder<'a>: Sharder
    where
        Self: 'a;

    fn sharder(&mut self) -> Self::Sharder<'_>;
}

/// Unified storage layer: byte-backed write access contract.
pub trait StorageMut: ChunkSourceMut {
    type Writer<'a>: CursorMut
    where
        Self: 'a;

    fn writer(&mut self) -> Self::Writer<'_>;
}

/// Cursor-based mutable writing contract.
pub trait CursorMut {
    fn pos(&self) -> usize;
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError>;
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError>;
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError>;
}

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

impl<S: StorageMut + ?Sized> StorageMut for &mut S {
    type Writer<'a>
        = S::Writer<'a>
    where
        Self: 'a;

    #[inline]
    fn writer(&mut self) -> Self::Writer<'_> {
        (**self).writer()
    }
}

impl<S: CursorMut + ?Sized> CursorMut for &mut S {
    #[inline]
    fn pos(&self) -> usize {
        (**self).pos()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        (**self).write(bytes)
    }

    #[inline]
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        (**self).align(alignment)
    }

    #[inline]
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        (**self).skip(len)
    }
}

impl ChunkSource for [u8] {
    #[inline]
    fn chunk_count(&self) -> usize {
        1
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        if idx == 0 { Some(self) } else { None }
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
    fn chunk_count(&self) -> usize {
        1
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        if idx == 0 {
            Some(self.as_slice())
        } else {
            None
        }
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
}

impl<'a> SliceSerializer<'a> {
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

impl<'a> ChunkSource for SliceSerializer<'a> {
    #[inline]
    fn chunk_count(&self) -> usize {
        1
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        if idx == 0 { Some(self.buf) } else { None }
    }
}

impl<'a> ChunkSourceMut for SliceSerializer<'a> {
    #[inline]
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        if idx == 0 { Some(self.buf) } else { None }
    }
}

impl StorageMut for SliceSerializer<'_> {
    type Writer<'b>
        = &'b mut Self
    where
        Self: 'b;

    fn writer(&mut self) -> Self::Writer<'_> {
        self
    }
}

impl CursorMut for SliceSerializer<'_> {
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
            byteops::copy_exact(&mut self.buf[start..end], &bytes[..len]);
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
    fn chunk_count(&self) -> usize {
        1
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        if idx == 0 {
            Some(self.buf.as_slice())
        } else {
            None
        }
    }
}

#[cfg(feature = "alloc")]
impl ChunkSourceMut for VecSerializer {
    #[inline]
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        if idx == 0 {
            Some(self.buf.as_mut_slice())
        } else {
            None
        }
    }
}

#[cfg(feature = "alloc")]
impl StorageMut for VecSerializer {
    type Writer<'b>
        = &'b mut Self
    where
        Self: 'b;

    fn writer(&mut self) -> Self::Writer<'_> {
        self
    }
}

#[cfg(feature = "alloc")]
impl CursorMut for VecSerializer {
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
