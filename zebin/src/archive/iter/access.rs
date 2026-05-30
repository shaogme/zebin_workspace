use core::marker::PhantomData;

#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::utils::chunk::Buf;
use crate::{prelude::*, validation::ValidationContext};

use super::{DummyContext, MAX_SEQUENCE_LEN};

#[path = "access/block_index.rs"]
mod block_index;
pub(crate) use block_index::{BlockIndex, deserialize_block_index};

#[cfg(feature = "alloc")]
pub(crate) use block_index::skip_block_index;

/// The archived representation of an iterator-based collection.
#[derive(Clone)]
pub struct ArchivedIter<A> {
    _marker: PhantomData<A>,
}

/// The read view of an iterator-based collection.
#[derive(Clone)]
pub struct ArchivedIterView<'a, A> {
    pub(crate) source: &'a [u8],
    pub(crate) start_pos: usize,
    pub(crate) len: usize,
    pub(crate) block_index: Option<BlockIndex>,
    pub(crate) _marker: PhantomData<A>,
}

impl<'a, A> ArchivedIterView<'a, A> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> ArchivedIterIter<'a, A>
    where
        A: Access,
    {
        ArchivedIterIter {
            cursor: OffsetSliceCursor::new(self.source, self.start_pos),
            remaining: self.len,
            _marker: PhantomData,
        }
    }

    /// O(1) random access to the *i*-th element.
    ///
    /// If a block index is present the lookup is constant-time (locate
    /// the block in O(1), scan at most `chunk_size` elements inside).
    /// Without a block index the method falls back to a full linear
    /// scan from the beginning.
    pub fn get(&self, index: usize) -> Result<A::View<'a>, AccessError>
    where
        A: Access,
    {
        if index >= self.len {
            return Err(AccessError::ValidationError {
                message: "ArchivedIter::get index out of bounds",
                pos: self.start_pos,
            });
        }

        if let Some(ref bi) = self.block_index {
            let block_idx = index / bi.chunk_size;
            let intra = index % bi.chunk_size;

            #[cfg(feature = "alloc")]
            let block_start = bi.block_offset(block_idx);
            #[cfg(not(feature = "alloc"))]
            let block_start = bi.block_offset_from_bytes(self.source, block_idx);

            if let Some(offset) = block_start {
                let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
                cursor.pos = offset;
                let mut ctx = DummyContext;
                // Skip `intra` elements, then deserialize the target.
                for _ in 0..intra {
                    let marker = cursor.read_u8(&mut ctx)?;
                    if marker != 1 {
                        return Err(AccessError::ValidationError {
                            message: "Invalid sequence marker during indexed access",
                            pos: cursor.pos() - 1,
                        });
                    }
                    if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                        cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
                    }
                    A::validate(&mut cursor, &mut ctx)?;
                }
                // Access the target element.
                let marker = cursor.read_u8(&mut ctx)?;
                if marker != 1 {
                    return Err(AccessError::ValidationError {
                        message: "Invalid sequence marker during indexed access",
                        pos: cursor.pos() - 1,
                    });
                }
                if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                    cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
                }
                return A::access(&mut cursor, &mut ctx);
            }
        }

        // Fallback: linear scan from the start.
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        let mut ctx = DummyContext;
        for _ in 0..index {
            let marker = cursor.read_u8(&mut ctx)?;
            if marker != 1 {
                return Err(AccessError::ValidationError {
                    message: "Invalid sequence marker",
                    pos: cursor.pos() - 1,
                });
            }
            if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
            }
            A::validate(&mut cursor, &mut ctx)?;
        }
        let marker = cursor.read_u8(&mut ctx)?;
        if marker != 1 {
            return Err(AccessError::ValidationError {
                message: "Invalid sequence marker",
                pos: cursor.pos() - 1,
            });
        }
        if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
            cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
        }
        A::access(&mut cursor, &mut ctx)
    }

    /// Create an iterator starting at `start` (0-indexed).
    ///
    /// If a block index is available the cursor is positioned at the
    /// containing block in O(1); otherwise a linear skip is performed.
    pub fn iter_from(&self, start: usize) -> ArchivedIterIter<'a, A>
    where
        A: Access,
    {
        if start >= self.len {
            return ArchivedIterIter {
                cursor: OffsetSliceCursor::new(self.source, self.start_pos),
                remaining: 0,
                _marker: PhantomData,
            };
        }

        let remaining = self.len - start;

        if let Some(ref bi) = self.block_index {
            let block_idx = start / bi.chunk_size;
            let intra = start % bi.chunk_size;

            #[cfg(feature = "alloc")]
            let block_start = bi.block_offset(block_idx);
            #[cfg(not(feature = "alloc"))]
            let block_start = bi.block_offset_from_bytes(self.source, block_idx);

            if let Some(offset) = block_start {
                let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
                cursor.pos = offset;
                let mut ctx = DummyContext;
                // Skip `intra` elements inside the block.
                for _ in 0..intra {
                    if let Ok(1) = cursor.read_u8(&mut ctx) {
                        if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                            let _ = cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx);
                        }
                        let _ = A::validate(&mut cursor, &mut ctx);
                    }
                }
                return ArchivedIterIter {
                    cursor,
                    remaining,
                    _marker: PhantomData,
                };
            }
        }

        // Fallback: linear skip.
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        let mut ctx = DummyContext;
        for _ in 0..start {
            if let Ok(1) = cursor.read_u8(&mut ctx) {
                if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                    let _ = cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx);
                }
                let _ = A::validate(&mut cursor, &mut ctx);
            }
        }
        ArchivedIterIter {
            cursor,
            remaining,
            _marker: PhantomData,
        }
    }
}

impl<A> ArchivedLayout for ArchivedIter<A>
where
    A: ArchivedLayout,
{
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
}

pub struct OffsetSliceCursor<'a> {
    slice: &'a [u8],
    offset: usize,
    pos: usize,
}

impl<'a> OffsetSliceCursor<'a> {
    #[inline]
    pub fn new(slice: &'a [u8], offset: usize) -> Self {
        Self {
            slice,
            offset,
            pos: 0,
        }
    }
}

impl<'a, 'b> Cursor<'b> for OffsetSliceCursor<'a>
where
    'a: 'b,
{
    #[inline]
    fn pos(&self) -> usize {
        self.pos + self.offset
    }

    #[inline]
    fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos()))?;
        if end > self.slice.len() {
            return Err(context.validation_error("Pointer out of bounds", self.pos()));
        }
        self.pos = end;
        Ok(())
    }

    #[inline]
    fn skip<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        self.advance(len, context)
    }

    #[inline]
    fn read_buf<C>(&mut self, len: usize, context: &mut C) -> Result<Buf<'b>, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.pos;
        self.advance(len, context)?;
        Ok(Buf::new(&self.slice[start..self.pos]))
    }

    #[inline]
    fn peek_buf<C>(&self, len: usize, context: &mut C) -> Result<Buf<'b>, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos()))?;
        if end > self.slice.len() {
            return Err(context.validation_error("Pointer out of bounds", self.pos()));
        }
        Ok(Buf::new(&self.slice[self.pos..end]))
    }

    #[inline]
    fn is_eof(&self) -> bool {
        self.pos >= self.slice.len()
    }
}

struct PeekingCursor<'b, Cr: ?Sized> {
    cursor: &'b mut Cr,
    peeked_len: usize,
}

impl<'b, 'a, Cr> Cursor<'a> for PeekingCursor<'b, Cr>
where
    Cr: Cursor<'a> + ?Sized,
{
    #[inline]
    fn pos(&self) -> usize {
        self.cursor.pos() + self.peeked_len
    }

    #[inline]
    fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let new_len = self
            .peeked_len
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos()))?;
        let _ = self.cursor.peek_buf(new_len, context)?;
        self.peeked_len = new_len;
        Ok(())
    }

    #[inline]
    fn skip<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        self.advance(len, context)
    }

    #[inline]
    fn read_buf<C>(&mut self, len: usize, context: &mut C) -> Result<Buf<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.peeked_len;
        self.advance(len, context)?;
        let full_buf = self.cursor.peek_buf(self.peeked_len, context)?;
        let slice = &full_buf.into_slice()[start..self.peeked_len];
        Ok(Buf::new(slice))
    }

    #[inline]
    fn peek_buf<C>(&self, len: usize, context: &mut C) -> Result<Buf<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.peeked_len;
        let end = start
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos()))?;
        let full_buf = self.cursor.peek_buf(end, context)?;
        let slice = &full_buf.into_slice()[start..end];
        Ok(Buf::new(slice))
    }

    #[inline]
    fn is_eof(&self) -> bool {
        let mut ctx = DummyContext;
        self.cursor.peek_buf(self.peeked_len + 1, &mut ctx).is_err()
    }
}

impl<A> Access for ArchivedIter<A>
where
    A: Access,
{
    type View<'a>
        = ArchivedIterView<'a, A>
    where
        Self: 'a;

    #[cfg(feature = "alloc")]
    type AccessStrategy = ForwardSequenceStrategy;

    fn access<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
        Self: 'a,
    {
        let start_pos = cursor.pos();
        let mut peeking_cursor = PeekingCursor {
            cursor,
            peeked_len: 0,
        };
        let len = Self::access_sequence_body(&mut peeking_cursor, context)?;

        // Try to parse trailing block index.
        let block_index = deserialize_block_index(&mut peeking_cursor, context, len)?;

        let total_len = peeking_cursor.peeked_len;
        let source_buf = cursor.read_buf(total_len, context)?;
        let source = source_buf.into_slice();

        Ok(ArchivedIterView {
            source,
            start_pos,
            len,
            block_index,
            _marker: PhantomData,
        })
    }

    fn validate<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        let len = Self::access_sequence_body(cursor, context)?;
        // Also consume block index bytes during validation.
        let _ = deserialize_block_index(cursor, context, len)?;
        Ok(())
    }
}

impl<A> ArchivedIter<A> {
    fn access_sequence_body<'a, C, Cr>(
        cursor: &mut Cr,
        context: &mut C,
    ) -> Result<usize, AccessError>
    where
        A: Access,
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        let mut len = 0usize;
        loop {
            let marker = cursor.read_u8(context)?;
            if marker == 0 {
                break;
            } else if marker != 1 {
                return Err(AccessError::ValidationError {
                    message: "Invalid sequence marker",
                    pos: cursor.pos() - 1,
                });
            }
            if len >= MAX_SEQUENCE_LEN {
                return Err(AccessError::ValidationError {
                    message: "Sequence length exceeds safety limit",
                    pos: cursor.pos() - 1,
                });
            }
            if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                cursor.align(<A as ArchivedLayout>::ALIGNMENT, context)?;
            }
            let mut guard = context.push_index(len);
            A::validate(cursor, &mut *guard)?;
            len += 1;
        }
        Ok(len)
    }
}

/// Lazy decoding iterator over the elements of an `ArchivedIter`.
pub struct ArchivedIterIter<'a, A> {
    pub(crate) cursor: OffsetSliceCursor<'a>,
    pub(crate) remaining: usize,
    pub(crate) _marker: PhantomData<A>,
}

impl<'a, A: Access + 'a> Iterator for ArchivedIterIter<'a, A> {
    type Item = Result<A::View<'a>, AccessError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let mut context = DummyContext;
        match self.cursor.read_u8(&mut context) {
            Ok(1) => {
                if <A as ArchivedLayout>::FIXED_SIZE.is_some()
                    && let Err(e) = self
                        .cursor
                        .align(<A as ArchivedLayout>::ALIGNMENT, &mut context)
                {
                    return Some(Err(e));
                }
                Some(A::access(&mut self.cursor, &mut context))
            }
            Ok(_) => Some(Err(AccessError::ValidationError {
                message: "Invalid sequence marker",
                pos: self.cursor.pos() - 1,
            })),
            Err(e) => Some(Err(e)),
        }
    }
}

impl<'a, A: 'a> ArchivedField<'a> for ArchivedIterView<'a, A> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archived_iter_empty() {
        let bytes = [0x00u8];
        let slice: &[u8] = &bytes;
        let iter: ArchivedIterView<'_, u8> = ArchivedIterView {
            source: slice,
            start_pos: 0,
            len: 0,
            block_index: None,
            _marker: PhantomData,
        };
        assert!(iter.is_empty());
        assert_eq!(iter.len(), 0);
        assert!(iter.get(0).is_err());
    }

    #[test]
    fn test_archived_iter_invalid_marker() {
        let bytes = [0x02u8, 0x00u8]; // Invalid marker
        let slice: &[u8] = &bytes;
        let iter: ArchivedIterView<'_, u32> = ArchivedIterView {
            source: slice,
            start_pos: 0,
            len: 1,
            block_index: None,
            _marker: PhantomData,
        };
        assert_eq!(iter.len(), 1);
        assert!(iter.get(0).is_err());
    }
}
