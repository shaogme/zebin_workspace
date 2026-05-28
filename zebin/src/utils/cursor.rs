use crate::prelude::*;
use crate::utils::chunk::{ChunkSource, ChunkSourceMut, ChunkedView, ChunkedViewMut};
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

        let buf = (*self.view.source).get_buf(start, len).map_err(|_| {
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
        if self.pos + len > self.limit {
            return Err(context.validation_error("Peek out of limit", self.pos));
        }
        context.check_range(self.pos, len)?;

        let buf = (*self.view.source).get_buf(self.pos, len).map_err(|_| {
            context.validation_error(
                "Requested range spans across non-contiguous chunks or out of bounds",
                self.pos,
            )
        })?;
        Ok(buf.into_slice())
    }
    // ... [rest of Cursor methods unchanged, view_file showed them] ...
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

/// Mutable cursor into an archive chunked view.
pub struct CursorMut<'a> {
    view: ChunkedViewMut<&'a mut (dyn ChunkSourceMut + 'a)>,
    pos: usize,
}

impl<'a> CursorMut<'a> {
    pub fn new(source: &'a mut (dyn ChunkSourceMut + 'a), pos: usize) -> Self {
        let view = ChunkedViewMut::new(source);
        Self { view, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn view(&self) -> &ChunkedViewMut<&'a mut (dyn ChunkSourceMut + 'a)> {
        &self.view
    }

    pub fn view_mut(&mut self) -> &mut ChunkedViewMut<&'a mut (dyn ChunkSourceMut + 'a)> {
        &mut self.view
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError> {
        if bytes.is_empty() {
            return Ok(SinkProgress::Complete);
        }
        let buf = (*self.view.source).get_buf_mut(self.pos, bytes.len())?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::copy_exact(buf.into_mut_slice(), &bytes[..accepted]);
        }
        let progress = SinkProgress::from_accepted(bytes.len(), accepted);
        self.pos += progress.accepted_for(bytes.len());
        self.view.total_len = self.view.source.total_len();
        Ok(progress)
    }

    pub fn align(
        &mut self,
        alignment: core::num::NonZeroUsize,
    ) -> Result<SinkProgress, ZebinError> {
        let padding = padding_for_alignment(self.pos, alignment);
        self.skip(padding)
    }

    pub fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError> {
        if len == 0 {
            return Ok(SinkProgress::Complete);
        }
        let buf = (*self.view.source).get_buf_mut(self.pos, len)?;
        let accepted = buf.len();
        if accepted > 0 {
            byteops::fill(buf.into_mut_slice(), 0);
        }
        let progress = SinkProgress::from_accepted(len, accepted);
        self.pos += progress.accepted_for(len);
        self.view.total_len = self.view.source.total_len();
        Ok(progress)
    }
}
