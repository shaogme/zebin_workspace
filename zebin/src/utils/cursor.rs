use crate::io::StorageMut;
use crate::prelude::*;
use crate::utils::chunk::ChunkSource;
use crate::utils::{byteops, padding_for_alignment};

/// Borrowed cursor into an archive byte slice.
pub struct Cursor<'a> {
    source: &'a (dyn ChunkSource + 'a),
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(source: &'a (dyn ChunkSource + 'a), pos: usize) -> Self {
        Self { source, pos }
    }

    pub fn source(&self) -> &'a (dyn ChunkSource + 'a) {
        self.source
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn with_pos(&self, pos: usize) -> Self {
        Self {
            source: self.source,
            pos,
        }
    }

    pub fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        context.check_range(self.pos, len)?;
        self.pos = end;
        Ok(())
    }

    pub fn align<C>(
        &mut self,
        alignment: core::num::NonZeroUsize,
        context: &mut C,
    ) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let padding = padding_for_alignment(self.pos, alignment);
        self.advance(padding, context)
    }

    pub fn read_exact<C>(&mut self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.pos;
        self.advance(len, context)?;

        let buf = (*self.source).get_buf(start, len).map_err(|_| {
            context.validation_error(
                "Requested range spans across non-contiguous chunks or out of bounds",
                start,
            )
        })?;
        Ok(buf.into_slice())
    }

    pub fn peek_exact<C>(&self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        context.check_range(self.pos, len)?;

        let buf = (*self.source).get_buf(self.pos, len).map_err(|_| {
            context.validation_error(
                "Requested range spans across non-contiguous chunks or out of bounds",
                self.pos,
            )
        })?;
        Ok(buf.into_slice())
    }

    pub fn read_array<const N: usize, C>(&mut self, context: &mut C) -> Result<[u8; N], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(N, context)?;
        let mut array = [0u8; N];
        byteops::copy_exact(&mut array, bytes);
        Ok(array)
    }

    pub fn read_u8<C>(&mut self, context: &mut C) -> Result<u8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(1, context)?;
        Ok(bytes[0])
    }

    pub fn read_u16<C>(&mut self, context: &mut C) -> Result<u16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(2, context)?;
        let mut array = [0u8; 2];
        byteops::copy_exact(&mut array, bytes);
        Ok(u16::from_le_bytes(array))
    }

    pub fn read_u32<C>(&mut self, context: &mut C) -> Result<u32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(4, context)?;
        let mut array = [0u8; 4];
        byteops::copy_exact(&mut array, bytes);
        Ok(u32::from_le_bytes(array))
    }

    pub fn read_i8<C>(&mut self, context: &mut C) -> Result<i8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(self.read_u8(context)? as i8)
    }

    pub fn read_i16<C>(&mut self, context: &mut C) -> Result<i16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(2, context)?;
        let mut array = [0u8; 2];
        byteops::copy_exact(&mut array, bytes);
        Ok(i16::from_le_bytes(array))
    }

    pub fn read_i32<C>(&mut self, context: &mut C) -> Result<i32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(4, context)?;
        let mut array = [0u8; 4];
        byteops::copy_exact(&mut array, bytes);
        Ok(i32::from_le_bytes(array))
    }

    pub fn read_u64<C>(&mut self, context: &mut C) -> Result<u64, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(8, context)?;
        let mut array = [0u8; 8];
        byteops::copy_exact(&mut array, bytes);
        Ok(u64::from_le_bytes(array))
    }

    pub fn read_i64<C>(&mut self, context: &mut C) -> Result<i64, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(8, context)?;
        let mut array = [0u8; 8];
        byteops::copy_exact(&mut array, bytes);
        Ok(i64::from_le_bytes(array))
    }
}

/// Writable cursor into an archive chunked view.
pub struct CursorMut<'a> {
    source: &'a mut (dyn StorageMut + 'a),
}

impl<'a> CursorMut<'a> {
    pub fn new(source: &'a mut (dyn StorageMut + 'a)) -> Self {
        Self { source }
    }

    pub fn pos(&self) -> usize {
        self.source.pos()
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let buf = (*self.source).peek_buf_mut(bytes.len())?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::copy_exact(buf.into_mut_slice(), &bytes[..accepted]);
            self.source.advance(accepted);
        }
        let progress = SinkProgress::from_accepted(bytes.len(), accepted);
        Ok(progress)
    }

    pub fn align(
        &mut self,
        alignment: core::num::NonZeroUsize,
    ) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.pos(), alignment);
        self.skip(padding)
    }

    pub fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let buf = (*self.source).peek_buf_mut(len)?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::fill(buf.into_mut_slice(), 0);
            self.source.advance(accepted);
        }
        let progress = SinkProgress::from_accepted(len, accepted);
        Ok(progress)
    }
}
