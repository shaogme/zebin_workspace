use core::num::NonZeroUsize;

#[cfg(feature = "mmap")]
use crate::io::MmapMut;
use crate::{
    prelude::*,
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

    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        let len = bytes.len();
        self.pos = self
            .pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        debug_assert!(len <= isize::MAX as usize);
        Ok(SinkProgress::Complete)
    }

    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let align_val = alignment.get();
        debug_assert!(align_val.is_power_of_two());
        let padding = padding_for_alignment(self.pos, alignment);
        self.pos = self
            .pos
            .checked_add(padding)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        debug_assert!(self.pos.is_multiple_of(alignment.get()));
        Ok(SinkProgress::Complete)
    }

    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        self.pos = self
            .pos
            .checked_add(len)
            .ok_or(ZebinError::ArithmeticOverflow { pos: self.pos })?;
        Ok(SinkProgress::Complete)
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

impl ByteSink for SliceEncoder<'_> {
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

#[cfg(feature = "mmap")]
/// Encoder that writes into a pre-sized memory-mapped file.
///
/// The mmap must be sized to fit the entire archive before construction.
/// All writes return `SinkProgress::Complete`; if a write would exceed the
/// map, `ZebinError::BufferTooSmall` is returned.
pub struct MmapEncoder {
    mmap: MmapMut,
    archive_pos: usize,
    written: usize,
}

#[cfg(feature = "mmap")]
impl MmapEncoder {
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

#[cfg(feature = "mmap")]
impl ByteSink for MmapEncoder {
    fn pos(&self) -> usize {
        self.archive_pos
    }

    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let (start, end) = self.prepare_range(bytes.len())?;
        self.mmap[start..end].copy_from_slice(bytes);
        Ok(SinkProgress::Complete)
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
        byteops::fill(&mut self.mmap[start..end], 0);
        Ok(SinkProgress::Complete)
    }
}
