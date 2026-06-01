use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use memmap2::{Mmap as RawMmap, MmapMut as RawMmapMut, MmapOptions};

use crate::error::ZebinError;
use crate::io::{Storage, StorageMut};
use crate::utils::chunk::BufMut;
use crate::utils::cursor::SliceCursor;

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

impl<'b> Storage for &'b Mmap {
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

    pub fn pos(&self) -> usize {
        self.archive_pos
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
}

impl StorageMut for MmapSerializer {
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
        if end > self.mmap.len() {
            return Err(ZebinError::BufferTooSmall {
                pos: self.archive_pos,
                required: end - self.mmap.len(),
            });
        }
        Ok(BufMut::new(&mut self.mmap[pos..end]))
    }

    #[inline]
    fn advance(&mut self, len: usize) {
        self.archive_pos = self.archive_pos.checked_add(len).expect("overflow");
        self.written = self.written.max(self.archive_pos);
    }
}
