use std::{fs::File, path::Path};

use memmap2::{Mmap as RawMmap, MmapOptions};

use crate::ZebinError;

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

impl crate::storage::Storage for Mmap {
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
