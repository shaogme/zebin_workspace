use core::num::NonZeroUsize;

use crate::{
    ZebinError,
    traits::ByteSink,
    utils::{byteops, padding_for_alignment},
};

/// Measuring encoder that simulates writes.
pub struct MeasureEncoder {
    pos: usize,
}

impl MeasureEncoder {
    pub fn new(pos: usize) -> Self {
        Self { pos }
    }
}

impl ByteSink for MeasureEncoder {
    fn pos(&self) -> usize {
        self.pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        let len = bytes.len();
        self.pos = self
            .pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        debug_assert!(len <= core::isize::MAX as usize);
        Ok(len)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let padding = padding_for_alignment(self.pos, alignment);
        self.pos = self
            .pos
            .checked_add(padding)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        debug_assert!(self.pos % alignment.get() == 0);
        Ok(padding)
    }

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError> {
        self.pos = self
            .pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        Ok(len)
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
        let start = self.written;
        let remaining = self.buf.len().saturating_sub(start);
        let count = remaining.min(len);

        if count == 0 && len > 0 {
            return Ok((0, 0));
        }

        let next_archive_pos =
            self.archive_pos
                .checked_add(count)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;

        let end = start + count;
        self.archive_pos = next_archive_pos;
        self.written = end;
        Ok((start, end))
    }
}

impl ByteSink for SliceEncoder<'_> {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        let (start, end) = self.prepare_range(bytes.len())?;
        let len = end - start;
        if len > 0 {
            self.buf[start..end].copy_from_slice(&bytes[..len]);
        }
        Ok(len)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let padding = padding_for_alignment(self.archive_pos, alignment);
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError> {
        let (start, end) = self.prepare_range(len)?;
        let written = end - start;
        if written > 0 {
            byteops::fill(&mut self.buf[start..end], 0);
        }
        Ok(written)
    }
}

#[cfg(feature = "alloc")]
/// Encoder that writes into a dynamically growing vector.
pub struct VecEncoder {
    buf: alloc::vec::Vec<u8>,
    archive_pos: usize,
}

#[cfg(feature = "alloc")]
impl VecEncoder {
    pub fn new(archive_pos: usize) -> Self {
        Self {
            buf: alloc::vec::Vec::new(),
            archive_pos,
        }
    }

    #[allow(dead_code)]
    pub fn with_capacity(capacity: usize, archive_pos: usize) -> Self {
        Self {
            buf: alloc::vec::Vec::with_capacity(capacity),
            archive_pos,
        }
    }

    pub fn into_inner(self) -> alloc::vec::Vec<u8> {
        self.buf
    }
}

#[cfg(feature = "alloc")]
impl ByteSink for VecEncoder {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        let next_pos =
            self.archive_pos
                .checked_add(bytes.len())
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        self.buf.extend_from_slice(bytes);
        self.archive_pos = next_pos;
        Ok(bytes.len())
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let padding = padding_for_alignment(self.archive_pos, alignment);
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError> {
        let next_pos = self
            .archive_pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow {
                pos: self.archive_pos,
            })?;
        self.buf.resize(self.buf.len() + len, 0);
        self.archive_pos = next_pos;
        Ok(len)
    }
}
