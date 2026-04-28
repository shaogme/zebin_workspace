#[cfg(feature = "mmap")]
pub mod mmap;

use alloc::{vec, vec::Vec};
use core::num::NonZeroUsize;

use crate::ZebinError;

/// Storage layer: byte-backed sequential write capabilities.
pub trait Storage {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn extend(&mut self, bytes: &[u8]) -> Result<usize, ZebinError>;
    fn align(&mut self, alignment: NonZeroUsize) -> Result<(), ZebinError> {
        let alignment = alignment.get();
        let pos = self.len();
        let padding = (alignment - (pos % alignment)) % alignment;
        if padding > 0 {
            let padding_bytes = vec![0u8; padding];
            self.extend(&padding_bytes)?;
        }
        Ok(())
    }
    fn into_bytes(self) -> Result<Vec<u8>, ZebinError>
    where
        Self: Sized;
}

impl Storage for Vec<u8> {
    fn len(&self) -> usize {
        self.len()
    }

    fn extend(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        self.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn into_bytes(self) -> Result<Vec<u8>, ZebinError> {
        Ok(self)
    }
}
