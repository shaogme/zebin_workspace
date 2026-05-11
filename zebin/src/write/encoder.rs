use core::num::NonZeroUsize;

use crate::{ZebinError, traits::ByteSink, utils::byteops};

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
        self.pos = self
            .pos
            .checked_add(bytes.len())
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        Ok(bytes.len())
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let alignment = alignment.get();
        let pos = self.pos;
        let padding = (alignment - (pos % alignment)) % alignment;
        self.pos = self
            .pos
            .checked_add(padding)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
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
}

impl ByteSink for SliceEncoder<'_> {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError> {
        let remaining = self.buf.len().saturating_sub(self.written);
        if remaining == 0 || bytes.is_empty() {
            return Ok(0);
        }

        let written = remaining.min(bytes.len());
        self.buf[self.written..self.written + written].copy_from_slice(&bytes[..written]);
        self.written += written;
        self.archive_pos =
            self.archive_pos
                .checked_add(written)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        Ok(written)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError> {
        let alignment = alignment.get();
        let padding = (alignment - (self.archive_pos % alignment)) % alignment;
        self.skip(padding)
    }

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError> {
        if len == 0 {
            return Ok(0);
        }

        let remaining = self.buf.len().saturating_sub(self.written);
        let written = remaining.min(len);
        if written > 0 {
            byteops::fill(&mut self.buf[self.written..self.written + written], 0);
            self.written += written;
        }
        self.archive_pos =
            self.archive_pos
                .checked_add(written)
                .ok_or(ZebinError::ArithmeticOverflow {
                    pos: self.archive_pos,
                })?;
        Ok(written)
    }
}
