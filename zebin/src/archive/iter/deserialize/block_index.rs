use crate::{archive_impl::varint::deserialize_u64, prelude::*, validation::ValidationContext};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::archive_impl::iter::BLOCK_INDEX_MAGIC;
#[cfg(any(not(feature = "alloc"), test))]
use crate::archive_impl::iter::DummyContext;

/// Sparse block index for fast random access into archived sequences.
///
/// Every `chunk_size` elements form a block.  The index stores the byte
/// offset of the first element marker in each block (relative to the
/// sequence `start_pos`).  To access element *i*, locate block
/// `i / chunk_size` in O(1) and scan at most `chunk_size` elements inside
/// that block.
#[cfg(not(feature = "alloc"))]
#[derive(Clone)]
pub struct BlockIndexNoAllocState {
    pub(crate) num_blocks: usize,
    pub(crate) raw_delta_start: usize,
}

#[derive(Clone)]
pub struct BlockIndex {
    pub(crate) chunk_size: usize,
    #[cfg(feature = "alloc")]
    pub(crate) offsets: Vec<usize>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) no_alloc_state: BlockIndexNoAllocState,
}

impl BlockIndex {
    /// Return the byte offset (relative to sequence start_pos) of block `block_idx`.
    #[cfg(feature = "alloc")]
    pub(crate) fn block_offset(&self, block_idx: usize) -> Option<usize> {
        self.offsets.get(block_idx).copied()
    }

    /// Return the byte offset by re-parsing varint deltas on the fly (no_alloc path).
    #[cfg(not(feature = "alloc"))]
    pub(crate) fn block_offset_from_bytes(&self, bytes: &[u8], block_idx: usize) -> Option<usize> {
        if block_idx >= self.no_alloc_state.num_blocks {
            return None;
        }
        let mut cursor = Cursor::new(bytes, self.no_alloc_state.raw_delta_start);
        let mut ctx = DummyContext;
        let mut abs = 0usize;
        for i in 0..=block_idx {
            match deserialize_u64::<usize, _>(&mut cursor, &mut ctx) {
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

/// Attempt to deserialize a block index section after the sequence sentinel.
///
/// Returns `None` when no index is present (fewer than `chunk_size + 1`
/// elements, or the magic byte is absent).
pub(crate) fn deserialize_block_index<C>(
    cursor: &mut Cursor<'_>,
    context: &mut C,
    len: usize,
) -> Result<Option<BlockIndex>, AccessError>
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
    let chunk_size: usize = deserialize_u64(cursor, context)?;
    if chunk_size == 0 {
        return Err(AccessError::ValidationError {
            message: "Block index chunk_size must be > 0",
            pos: cursor.pos(),
        });
    }

    // num_blocks (varint)
    let num_blocks: usize = deserialize_u64(cursor, context)?;
    let expected_blocks = len.div_ceil(chunk_size);
    if num_blocks != expected_blocks {
        return Err(AccessError::ValidationError {
            message: "Block index num_blocks mismatch",
            pos: cursor.pos(),
        });
    }

    #[cfg(feature = "alloc")]
    {
        let mut offsets = Vec::with_capacity(num_blocks);
        let mut abs: usize = 0;
        for _ in 0..num_blocks {
            let delta: usize = deserialize_u64(cursor, context)?;
            abs += delta;
            offsets.push(abs);
        }
        Ok(Some(BlockIndex {
            chunk_size,
            offsets,
        }))
    }

    #[cfg(not(feature = "alloc"))]
    {
        let raw_delta_start = cursor.pos();
        // Walk through all delta varints to advance the cursor past them.
        for _ in 0..num_blocks {
            let _delta: usize = deserialize_u64(cursor, context)?;
        }
        Ok(Some(BlockIndex {
            chunk_size,
            no_alloc_state: BlockIndexNoAllocState {
                num_blocks,
                raw_delta_start,
            },
        }))
    }
}

/// Skip a block index section without storing it (used by
/// `SequenceAccessStrategy` impls that eagerly deserialize everything).
#[cfg(feature = "alloc")]
pub(crate) fn skip_block_index<C>(
    cursor: &mut Cursor<'_>,
    context: &mut C,
) -> Result<(), AccessError>
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

    let _chunk_size: usize = deserialize_u64(cursor, context)?;
    let num_blocks: usize = deserialize_u64(cursor, context)?;
    for _ in 0..num_blocks {
        let _delta: usize = deserialize_u64(cursor, context)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_varint(value: u64, buf: &mut Vec<u8>) {
        use crate::archive_impl::varint::{serialize_u64, serialized_len_u64};
        let len = serialized_len_u64(value);
        let start = buf.len();
        buf.resize(start + len, 0);
        serialize_u64(value, &mut buf[start..]);
    }

    #[test]
    fn test_block_offset_empty_or_out_of_bounds() {
        #[cfg(feature = "alloc")]
        {
            let bi = BlockIndex {
                chunk_size: 64,
                offsets: vec![10, 20, 30],
            };
            assert_eq!(bi.block_offset(0), Some(10));
            assert_eq!(bi.block_offset(1), Some(20));
            assert_eq!(bi.block_offset(2), Some(30));
            assert_eq!(bi.block_offset(3), None);
        }

        #[cfg(not(feature = "alloc"))]
        {
            let mut bytes = Vec::new();
            append_varint(10, &mut bytes);
            append_varint(10, &mut bytes);
            append_varint(10, &mut bytes);

            let bi = BlockIndex {
                chunk_size: 64,
                no_alloc_state: BlockIndexNoAllocState {
                    num_blocks: 3,
                    raw_delta_start: 0,
                },
            };
            assert_eq!(bi.block_offset_from_bytes(&bytes, 0), Some(10));
            assert_eq!(bi.block_offset_from_bytes(&bytes, 1), Some(20));
            assert_eq!(bi.block_offset_from_bytes(&bytes, 2), Some(30));
            assert_eq!(bi.block_offset_from_bytes(&bytes, 3), None);
        }
    }

    #[test]
    fn test_deserialize_block_index_missing_magic() {
        let data = [0x00, 0x01, 0x02];
        let mut cursor = Cursor::new(&data, 0);
        let mut ctx = DummyContext;
        let res = deserialize_block_index(&mut cursor, &mut ctx, 10);
        assert!(res.is_ok());
        assert!(res.unwrap().is_none());
    }

    #[test]
    fn test_deserialize_block_index_invalid_chunk_size() {
        let mut data = Vec::new();
        data.push(BLOCK_INDEX_MAGIC);
        append_varint(0, &mut data); // chunk_size = 0
        append_varint(2, &mut data); // num_blocks = 2

        let mut cursor = Cursor::new(&data, 0);
        let mut ctx = DummyContext;
        let res = deserialize_block_index(&mut cursor, &mut ctx, 10);
        assert!(res.is_err());
    }

    #[test]
    fn test_deserialize_block_index_mismatch_blocks() {
        let mut data = Vec::new();
        data.push(BLOCK_INDEX_MAGIC);
        append_varint(64, &mut data); // chunk_size = 64
        append_varint(5, &mut data); // num_blocks = 5, but len = 10, expected blocks = 1

        let mut cursor = Cursor::new(&data, 0);
        let mut ctx = DummyContext;
        let res = deserialize_block_index(&mut cursor, &mut ctx, 10);
        assert!(res.is_err());
    }
}
