use crate::prelude::*;
use crate::utils::chunk::{ChunkSource, ChunkedView};
use crate::utils::{byteops, padding_for_alignment};

/// Borrowed cursor into an archive byte slice.
pub struct Cursor<'a> {
    view: ChunkedView<&'a (dyn ChunkSource + 'a)>,
    pos: usize,
    limit: usize,
}

impl<'a> Clone for Cursor<'a> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            view: self.view.clone(),
            pos: self.pos,
            limit: self.limit,
        }
    }
}

impl<'a> Cursor<'a> {
    pub fn new(source: &'a (dyn ChunkSource + 'a), pos: usize) -> Self {
        let view = ChunkedView::new_ref(source);
        let limit = view.len();
        Self { view, pos, limit }
    }

    pub fn new_with_limit(source: &'a (dyn ChunkSource + 'a), pos: usize, limit: usize) -> Self {
        Self {
            view: ChunkedView::new_ref(source),
            pos,
            limit,
        }
    }

    pub fn view(&self) -> &ChunkedView<&'a (dyn ChunkSource + 'a)> {
        &self.view
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.pos)
    }

    pub fn with_pos(&self, pos: usize) -> Self {
        Self {
            view: self.view.clone(),
            pos,
            limit: self.limit,
        }
    }

    pub fn with_limit(&self, limit: usize) -> Self {
        Self {
            view: self.view.clone(),
            pos: self.pos,
            limit,
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
        if end > self.limit {
            return Err(context.validation_error("Cursor advance out of limit", self.pos));
        }
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

        let (chunk_idx, local_idx) = self
            .view
            .translate_address(start)
            .ok_or_else(|| context.validation_error("Address translation failed", start))?;

        let chunk = self.view.source.get_chunk(chunk_idx).unwrap();
        if local_idx + len <= chunk.len() {
            Ok(&chunk[local_idx..local_idx + len])
        } else {
            Err(context
                .validation_error("Requested range spans across non-contiguous chunks", start))
        }
    }

    pub fn peek_exact<C>(&self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        if self.pos + len > self.limit {
            return Err(context.validation_error("Peek out of limit", self.pos));
        }
        context.check_range(self.pos, len)?;

        let (chunk_idx, local_idx) = self
            .view
            .translate_address(self.pos)
            .ok_or_else(|| context.validation_error("Address translation failed", self.pos))?;

        let chunk = self.view.source.get_chunk(chunk_idx).unwrap();
        if local_idx + len <= chunk.len() {
            Ok(&chunk[local_idx..local_idx + len])
        } else {
            Err(context.validation_error(
                "Requested range spans across non-contiguous chunks",
                self.pos,
            ))
        }
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
