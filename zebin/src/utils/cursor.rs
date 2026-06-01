use core::num::NonZeroUsize;

use crate::error::ZebinError;
use crate::prelude::*;
use crate::traits_impl::SinkProgress;
use crate::utils::{byteops, padding_for_alignment};
use crate::validation::ValidationContext;

pub trait Cursor<'a> {
    fn pos(&self) -> usize;
    fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized;

    fn read_buf<C>(&mut self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let buf = self.peek_buf(len, context)?;
        self.advance(len, context)?;
        Ok(buf)
    }

    fn peek_buf<C>(&self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized;

    fn is_eof(&self) -> bool;

    #[inline]
    fn check_range<C>(&self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        self.peek_buf(len, context).map(|_| ())
    }

    #[inline]
    fn align<C>(&mut self, alignment: NonZeroUsize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let padding = padding_for_alignment(self.pos(), alignment);
        self.advance(padding, context)
    }

    #[inline]
    fn read_exact<C>(&mut self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        self.read_buf(len, context)
    }

    #[inline]
    fn peek_exact<C>(&self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        self.peek_buf(len, context)
    }

    #[inline]
    fn read_array<const N: usize, C>(&mut self, context: &mut C) -> Result<[u8; N], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(N, context)?;
        let mut array = [0u8; N];
        byteops::copy_exact(&mut array, bytes);
        Ok(array)
    }

    #[inline]
    fn read_u8<C>(&mut self, context: &mut C) -> Result<u8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(1, context)?;
        Ok(bytes[0])
    }

    #[inline]
    fn read_u16<C>(&mut self, context: &mut C) -> Result<u16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(2, context)?;
        let mut array = [0u8; 2];
        byteops::copy_exact(&mut array, bytes);
        Ok(u16::from_le_bytes(array))
    }

    #[inline]
    fn read_u32<C>(&mut self, context: &mut C) -> Result<u32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(4, context)?;
        let mut array = [0u8; 4];
        byteops::copy_exact(&mut array, bytes);
        Ok(u32::from_le_bytes(array))
    }

    #[inline]
    fn read_i8<C>(&mut self, context: &mut C) -> Result<i8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        self.read_u8(context).map(|b| b as i8)
    }

    #[inline]
    fn read_i16<C>(&mut self, context: &mut C) -> Result<i16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(2, context)?;
        let mut array = [0u8; 2];
        byteops::copy_exact(&mut array, bytes);
        Ok(i16::from_le_bytes(array))
    }

    #[inline]
    fn read_i32<C>(&mut self, context: &mut C) -> Result<i32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(4, context)?;
        let mut array = [0u8; 4];
        byteops::copy_exact(&mut array, bytes);
        Ok(i32::from_le_bytes(array))
    }

    #[inline]
    fn read_u64<C>(&mut self, context: &mut C) -> Result<u64, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(8, context)?;
        let mut array = [0u8; 8];
        byteops::copy_exact(&mut array, bytes);
        Ok(u64::from_le_bytes(array))
    }

    #[inline]
    fn read_i64<C>(&mut self, context: &mut C) -> Result<i64, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes = self.read_exact(8, context)?;
        let mut array = [0u8; 8];
        byteops::copy_exact(&mut array, bytes);
        Ok(i64::from_le_bytes(array))
    }
}

pub struct SliceCursor<'a> {
    slice: &'a [u8],
    pos: usize,
}

impl<'a> SliceCursor<'a> {
    #[inline]
    pub fn new(slice: &'a [u8], pos: usize) -> Self {
        Self { slice, pos }
    }
}

impl<'a, 'b> Cursor<'b> for SliceCursor<'a>
where
    'a: 'b,
{
    #[inline]
    fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        if end > self.slice.len() {
            return Err(context.validation_error("Pointer out of bounds", self.pos));
        }
        self.pos = end;
        Ok(())
    }

    #[inline]
    fn peek_buf<C>(&self, len: usize, context: &mut C) -> Result<&'b [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        if end > self.slice.len() {
            return Err(context.validation_error("Pointer out of bounds", self.pos));
        }
        Ok(&self.slice[self.pos..end])
    }

    #[inline]
    fn is_eof(&self) -> bool {
        self.pos >= self.slice.len()
    }
}

/// Writable cursor into an archive chunked view.
pub trait CursorMut<'a> {
    fn pos(&self) -> usize;
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError>;
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError>;
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError>;
}

impl<'a, C: CursorMut<'a> + ?Sized> CursorMut<'a> for &mut C {
    #[inline]
    fn pos(&self) -> usize {
        (**self).pos()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        (**self).write(bytes)
    }

    #[inline]
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        (**self).align(alignment)
    }

    #[inline]
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        (**self).skip(len)
    }
}

pub trait ChunkSerializer {
    fn pos(&self) -> usize;
    fn peek_buf_mut(&mut self, len: usize) -> Result<&mut [u8], ZebinError>;
    fn advance(&mut self, len: usize);
}

pub struct SerializerCursor<'a, S> {
    serializer: &'a mut S,
}

impl<'a, S> SerializerCursor<'a, S> {
    #[inline]
    pub fn new(serializer: &'a mut S) -> Self {
        Self { serializer }
    }

    #[inline]
    pub fn into_inner(self) -> &'a mut S {
        self.serializer
    }
}

impl<'a, 'b, S: ChunkSerializer> CursorMut<'b> for SerializerCursor<'a, S>
where
    'a: 'b,
{
    #[inline]
    fn pos(&self) -> usize {
        self.serializer.pos()
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let buf = self.serializer.peek_buf_mut(bytes.len())?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::copy_exact(buf, &bytes[..accepted]);
            self.serializer.advance(accepted);
        }
        Ok(SinkProgress::from_accepted(bytes.len(), accepted))
    }

    #[inline]
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.pos(), alignment);
        self.skip(padding)
    }

    #[inline]
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let buf = self.serializer.peek_buf_mut(len)?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::fill(buf, 0);
            self.serializer.advance(accepted);
        }
        Ok(SinkProgress::from_accepted(len, accepted))
    }
}
