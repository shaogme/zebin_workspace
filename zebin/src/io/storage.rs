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

/// Unified storage layer: byte-backed read access contract.
pub trait Storage {
    fn as_slice(&self) -> &[u8];
}

/// Unified storage layer: byte-backed write access contract.
pub trait StorageMut {
    fn pos(&self) -> usize;
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError>;
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError>;
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError>;
}

impl Storage for &[u8] {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        self
    }
}

impl Storage for [u8] {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        self
    }
}

#[cfg(feature = "alloc")]
impl Storage for Vec<u8> {
    #[inline]
    fn as_slice(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Chunked encoder that writes into a caller-provided buffer slice.
pub struct SliceEncoder<'a> {
    buf: &'a mut [u8],
    written: usize,
    archive_pos: usize,
}

impl<'a> SliceEncoder<'a> {
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

impl StorageMut for SliceEncoder<'_> {
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
            self.buf[start..end].copy_from_slice(&bytes[..len]);
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
/// Encoder that writes into a dynamically growing vector.
pub struct VecEncoder {
    buf: Vec<u8>,
    archive_pos: usize,
}

#[cfg(feature = "alloc")]
impl VecEncoder {
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
impl StorageMut for VecEncoder {
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
