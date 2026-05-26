use core::marker::PhantomData;

use crate::{
    prelude::*,
    validation::ValidationContext,
};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::{DummyContext, BLOCK_INDEX_MAGIC, MAX_SEQUENCE_LEN};

/// Sparse block index for fast random access into archived sequences.
///
/// Every `chunk_size` elements form a block.  The index stores the byte
/// offset of the first element marker in each block (relative to the
/// sequence `start_pos`).  To access element *i*, locate block
/// `i / chunk_size` in O(1) and scan at most `chunk_size` elements inside
/// that block.
#[derive(Clone)]
pub struct BlockIndex {
    pub(crate) chunk_size: usize,
    #[cfg(not(feature = "alloc"))]
    pub(crate) num_blocks: usize,
    /// Decoded absolute byte offsets for each block, relative to sequence
    /// `start_pos`.
    ///
    /// Under `no_alloc`, this is replaced by raw byte-range fields so
    /// offsets can be decoded on the fly.
    #[cfg(feature = "alloc")]
    pub(crate) offsets: Vec<usize>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) raw_delta_start: usize,
}

impl BlockIndex {
    /// Return the byte offset (relative to sequence start_pos) of block `block_idx`.
    #[cfg(feature = "alloc")]
    fn block_offset(&self, block_idx: usize) -> Option<usize> {
        self.offsets.get(block_idx).copied()
    }

    /// Return the byte offset by re-parsing varint deltas on the fly (no_alloc path).
    #[cfg(not(feature = "alloc"))]
    fn block_offset_from_bytes(&self, bytes: &[u8], block_idx: usize) -> Option<usize> {
        if block_idx >= self.num_blocks {
            return None;
        }
        let mut cursor = Cursor::new(bytes, self.raw_delta_start);
        let mut ctx = DummyContext;
        let mut abs = 0usize;
        for i in 0..=block_idx {
            match super::super::varint::decode_u64::<usize, _>(&mut cursor, &mut ctx) {
                Ok(delta) => {
                    abs += delta;
                }
                Err(_) => return None,
            }
            if i == block_idx {
                return Some(abs);
            }
        }
        None
    }
}

/// The archived representation of an iterator-based collection.
#[derive(Clone)]
pub struct ArchivedIter<'a, A> {
    pub(crate) bytes: &'a [u8],
    pub(crate) start_pos: usize,
    pub(crate) len: usize,
    pub(crate) block_index: Option<BlockIndex>,
    pub(crate) _marker: PhantomData<A>,
}

impl<'a, A> ArchivedIter<'a, A> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn iter(&self) -> ArchivedIterIter<'a, A>
    where
        A: Decode<'a>,
    {
        ArchivedIterIter {
            cursor: Cursor::new(self.bytes, self.start_pos),
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
    pub fn get(&self, index: usize) -> Result<A::View, DecodeError>
    where
        A: Decode<'a>,
    {
        if index >= self.len {
            return Err(DecodeError::ValidationError {
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
            let block_start = bi.block_offset_from_bytes(self.bytes, block_idx);

            if let Some(offset) = block_start {
                let abs_pos = self.start_pos + offset;
                let mut cursor = Cursor::new(self.bytes, abs_pos);
                let mut ctx = DummyContext;
                // Skip `intra` elements, then decode the target.
                for _ in 0..intra {
                    let marker = cursor.read_u8(&mut ctx)?;
                    if marker != 1 {
                        return Err(DecodeError::ValidationError {
                            message: "Invalid sequence marker during indexed access",
                            pos: cursor.pos() - 1,
                        });
                    }
                    if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                        cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
                    }
                    A::validate(&mut cursor, &mut ctx)?;
                }
                // Decode the target element.
                let marker = cursor.read_u8(&mut ctx)?;
                if marker != 1 {
                    return Err(DecodeError::ValidationError {
                        message: "Invalid sequence marker during indexed access",
                        pos: cursor.pos() - 1,
                    });
                }
                if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                    cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
                }
                return A::decode(&mut cursor, &mut ctx);
            }
        }

        // Fallback: linear scan from the start.
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        let mut ctx = DummyContext;
        for _ in 0..index {
            let marker = cursor.read_u8(&mut ctx)?;
            if marker != 1 {
                return Err(DecodeError::ValidationError {
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
            return Err(DecodeError::ValidationError {
                message: "Invalid sequence marker",
                pos: cursor.pos() - 1,
            });
        }
        if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
            cursor.align(<A as ArchivedLayout>::ALIGNMENT, &mut ctx)?;
        }
        A::decode(&mut cursor, &mut ctx)
    }

    /// Create an iterator starting at `start` (0-indexed).
    ///
    /// If a block index is available the cursor is positioned at the
    /// containing block in O(1); otherwise a linear skip is performed.
    pub fn iter_from(&self, start: usize) -> ArchivedIterIter<'a, A>
    where
        A: Decode<'a>,
    {
        if start >= self.len {
            return ArchivedIterIter {
                cursor: Cursor::new(self.bytes, self.start_pos),
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
            let block_start = bi.block_offset_from_bytes(self.bytes, block_idx);

            if let Some(offset) = block_start {
                let abs_pos = self.start_pos + offset;
                let mut cursor = Cursor::new(self.bytes, abs_pos);
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
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
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

impl<A> ArchivedLayout for ArchivedIter<'_, A>
where
    A: ArchivedLayout,
{
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
}

impl<'marker, 'a, A> Decode<'a> for ArchivedIter<'marker, A>
where
    A: Decode<'a> + 'a,
{
    type View = ArchivedIter<'a, A>;

    #[cfg(feature = "alloc")]
    type DecodeStrategy = crate::io::ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let start_pos = cursor.pos();
        let len = Self::decode_sequence_body(cursor, context)?;

        // Try to parse trailing block index.
        let block_index = decode_block_index(cursor, context, len)?;

        Ok(ArchivedIter {
            bytes: cursor.bytes(),
            start_pos,
            len,
            block_index,
            _marker: PhantomData,
        })
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = Self::decode_sequence_body(cursor, context)?;
        // Also consume block index bytes during validation.
        let _ = decode_block_index::<C>(cursor, context, len)?;
        Ok(())
    }
}

impl<'marker, A> ArchivedIter<'marker, A> {
    fn decode_sequence_body<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<usize, DecodeError>
    where
        A: Decode<'a>,
        C: ValidationContext + ?Sized,
    {
        let mut len = 0usize;
        loop {
            let marker = cursor.read_u8(context)?;
            if marker == 0 {
                break;
            } else if marker != 1 {
                return Err(DecodeError::ValidationError {
                    message: "Invalid sequence marker",
                    pos: cursor.pos() - 1,
                });
            }
            if len >= MAX_SEQUENCE_LEN {
                return Err(DecodeError::ValidationError {
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

/// Attempt to decode a block index section after the sequence sentinel.
///
/// Returns `None` when no index is present (fewer than `chunk_size + 1`
/// elements, or the magic byte is absent).
pub(crate) fn decode_block_index<C>(
    cursor: &mut Cursor<'_>,
    context: &mut C,
    len: usize,
) -> Result<Option<BlockIndex>, DecodeError>
where
    C: ValidationContext + ?Sized,
{
    // No index written for short sequences.
    if cursor.remaining() == 0 {
        return Ok(None);
    }

    // Peek at the next byte – if it isn't the magic, there is no index.
    let peeked = cursor.peek_exact(1, context)?;
    if peeked[0] != BLOCK_INDEX_MAGIC {
        return Ok(None);
    }
    // Consume the magic byte.
    cursor.advance(1, context)?;

    // chunk_size (varint)
    let chunk_size: usize = super::super::varint::decode_u64(cursor, context)?;
    if chunk_size == 0 {
        return Err(DecodeError::ValidationError {
            message: "Block index chunk_size must be > 0",
            pos: cursor.pos(),
        });
    }

    // num_blocks (varint)
    let num_blocks: usize = super::super::varint::decode_u64(cursor, context)?;
    let expected_blocks = len.div_ceil(chunk_size);
    if num_blocks != expected_blocks {
        return Err(DecodeError::ValidationError {
            message: "Block index num_blocks mismatch",
            pos: cursor.pos(),
        });
    }

    #[cfg(feature = "alloc")]
    {
        let mut offsets = Vec::with_capacity(num_blocks);
        let mut abs: usize = 0;
        for _ in 0..num_blocks {
            let delta: usize = super::super::varint::decode_u64(cursor, context)?;
            abs += delta;
            offsets.push(abs);
        }
        Ok(Some(BlockIndex {
            chunk_size,
            #[cfg(not(feature = "alloc"))]
            num_blocks,
            offsets,
        }))
    }

    #[cfg(not(feature = "alloc"))]
    {
        let raw_delta_start = cursor.pos();
        // Walk through all delta varints to advance the cursor past them.
        for _ in 0..num_blocks {
            let _delta: usize = super::super::varint::decode_u64(cursor, context)?;
        }
        Ok(Some(BlockIndex {
            chunk_size,
            num_blocks,
            raw_delta_start,
        }))
    }
}

/// Skip a block index section without storing it (used by
/// `SequenceDecodeStrategy` impls that eagerly decode everything).
#[cfg(feature = "alloc")]
pub(crate) fn skip_block_index<C>(
    cursor: &mut Cursor<'_>,
    context: &mut C,
) -> Result<(), DecodeError>
where
    C: ValidationContext + ?Sized,
{
    if cursor.remaining() == 0 {
        return Ok(());
    }
    let peeked = cursor.peek_exact(1, context)?;
    if peeked[0] != BLOCK_INDEX_MAGIC {
        return Ok(());
    }
    cursor.advance(1, context)?;

    let _chunk_size: usize = super::super::varint::decode_u64(cursor, context)?;
    let num_blocks: usize = super::super::varint::decode_u64(cursor, context)?;
    for _ in 0..num_blocks {
        let _delta: usize = super::super::varint::decode_u64(cursor, context)?;
    }
    Ok(())
}

/// Lazy decoding iterator over the elements of an `ArchivedIter`.
pub struct ArchivedIterIter<'a, A: Decode<'a>> {
    pub(crate) cursor: Cursor<'a>,
    pub(crate) remaining: usize,
    pub(crate) _marker: PhantomData<A>,
}

impl<'a, A: Decode<'a>> Iterator for ArchivedIterIter<'a, A> {
    type Item = Result<A::View, DecodeError>;

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
                Some(A::decode(&mut self.cursor, &mut context))
            }
            Ok(_) => Some(Err(DecodeError::ValidationError {
                message: "Invalid sequence marker",
                pos: self.cursor.pos() - 1,
            })),
            Err(e) => Some(Err(e)),
        }
    }
}
