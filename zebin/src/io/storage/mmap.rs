use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use memmap2::{Mmap as RawMmap, MmapMut as RawMmapMut, MmapOptions};

use crate::error::ZebinError;
use crate::io::{CursorMut, Storage, StorageMut};
use crate::traits_impl::SinkProgress;
use crate::utils::{byteops, padding_for_alignment};
use core::num::NonZeroUsize;

/// Memory-mapped storage backend for read-only archive access.
pub struct Mmap {
    data: RawMmap,
}

impl Mmap {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let data = unsafe { MmapOptions::new().map(&file)? };
        Ok(Self { data })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn extend(&mut self, _bytes: &[u8]) -> Result<usize, ZebinError> {
        Err(ZebinError::ReadOnlyStorage)
    }
}

use crate::utils::chunk::{ChunkSource, ChunkSourceMut};

impl ChunkSource for Mmap {
    #[inline]
    fn chunk_count(&self) -> usize {
        1
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        if idx == 0 { Some(&self.data) } else { None }
    }
}

impl Storage for Mmap {
    type Mode = crate::io::StaticMode;
    type Sharder<'a>
        = crate::io::NoSharder
    where
        Self: 'a;

    #[inline]
    fn sharder(&mut self) -> Self::Sharder<'_> {
        crate::io::NoSharder
    }
}

/// Writable memory-mapped storage backend used by [`MmapSerializer`].
///
/// Wraps [`memmap2::MmapMut`] so callers don't need a direct dependency on
/// `memmap2`. Dereferences to `[u8]` for indexing and slicing.
pub struct MmapMut {
    data: RawMmapMut,
}

impl MmapMut {
    /// Create (or truncate) `path`, size it to `len` bytes, and map it writable.
    pub fn create(path: impl AsRef<Path>, len: u64) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(len)?;
        let data = unsafe { MmapOptions::new().map_mut(&file)? };
        Ok(Self { data })
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn flush(&self) -> std::io::Result<()> {
        self.data.flush()
    }
}

impl core::ops::Deref for MmapMut {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.data
    }
}

impl core::ops::DerefMut for MmapMut {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

/// Serializer that writes into a pre-sized memory-mapped file.
///
/// The mmap must be sized to fit the entire archive before construction.
/// All writes return `SinkProgress::Complete`; if a write would exceed the
/// map, `ZebinError::BufferTooSmall` is returned.
pub struct MmapSerializer {
    mmap: MmapMut,
    archive_pos: usize,
    written: usize,
}

impl MmapSerializer {
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

impl ChunkSource for MmapSerializer {
    #[inline]
    fn chunk_count(&self) -> usize {
        1
    }

    #[inline]
    fn get_chunk(&self, idx: usize) -> Option<&[u8]> {
        if idx == 0 { Some(&self.mmap[..]) } else { None }
    }
}

impl ChunkSourceMut for MmapSerializer {
    #[inline]
    fn get_chunk_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        if idx == 0 {
            Some(&mut self.mmap[..])
        } else {
            None
        }
    }

    fn write_at(&mut self, _pos: usize, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(bytes.len())?;
        byteops::copy_exact(&mut self.mmap[start..end], bytes);
        Ok(SinkProgress::Complete)
    }

    fn align_at(
        &mut self,
        _pos: usize,
        alignment: NonZeroUsize,
    ) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.archive_pos, alignment);
        self.skip_at(_pos, padding)
    }

    fn skip_at(&mut self, _pos: usize, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(len)?;
        byteops::fill(&mut self.mmap[start..end], 0);
        Ok(SinkProgress::Complete)
    }
}

impl StorageMut for MmapSerializer {
    fn writer(&mut self) -> CursorMut<'_> {
        let pos = self.archive_pos;
        CursorMut::new(self, pos)
    }
}
