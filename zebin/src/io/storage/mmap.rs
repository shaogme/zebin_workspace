use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use memmap2::{Mmap as RawMmap, MmapMut as RawMmapMut, MmapOptions};

use crate::error::ZebinError;
use crate::io::storage::Storage;

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
}

impl Storage for Mmap {
    fn len(&self) -> usize {
        self.data.len()
    }

    fn extend(&mut self, _bytes: &[u8]) -> Result<usize, ZebinError> {
        Err(ZebinError::ReadOnlyStorage)
    }

    fn into_bytes(self) -> Result<Vec<u8>, ZebinError> {
        Ok(self.data.as_ref().to_vec())
    }
}

/// Writable memory-mapped storage backend used by [`crate::MmapEncoder`].
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
