use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::task::Poll;

use crate::{
    prelude::*,
    validation::{ValidationContext, ValidationPathSegment},
};

#[cfg(feature = "alloc")]
use alloc::{
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    vec::Vec,
};

#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

struct DummyContext;

impl ValidationContext for DummyContext {
    fn push_depth(&mut self) -> Result<(), DecodeError> {
        Ok(())
    }

    fn pop_depth(&mut self) {}

    fn push_path(&mut self, _segment: ValidationPathSegment) {}

    fn pop_path(&mut self) {}

    fn record_error_path(&mut self) {}

    fn check_range(&mut self, _pos: usize, _size: usize) -> Result<(), DecodeError> {
        Ok(())
    }

    fn check_alignment(
        &mut self,
        _pos: usize,
        _alignment: NonZeroUsize,
    ) -> Result<(), DecodeError> {
        Ok(())
    }

    fn check_sequence_len(&mut self, _len: usize, _pos: usize) -> Result<(), DecodeError> {
        Ok(())
    }
}

/// Maximum number of elements accepted in a single sequence during
/// decoding.  Prevents maliciously crafted input from triggering
/// integer-overflow or denial-of-service via extremely large `len` fields.
const MAX_SEQUENCE_LEN: usize = 1 << 28; // 256 Mi elements

/// Default number of elements per block in the sparse index.
const DEFAULT_CHUNK_SIZE: usize = 64;

/// Magic byte appended after the block index section to signal its presence.
/// Chosen to not conflict with the sequence marker (0x01) or sentinel (0x00).
const BLOCK_INDEX_MAGIC: u8 = 0x42;

/// Wrapper to enable encoding support for arbitrary types that implement `IntoIterator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IterArchive<I, T>(pub I, pub PhantomData<T>);

impl<I, T> IterArchive<I, T> {
    pub fn new(inner: I) -> Self {
        Self(inner, PhantomData)
    }

    pub fn into_inner(self) -> I {
        self.0
    }
}

impl<I, T> IntoIterator for IterArchive<I, T>
where
    I: IntoIterator<Item = T>,
{
    type Item = T;
    type IntoIter = I::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<I, T> Archive for IterArchive<I, T>
where
    for<'a> &'a I: IntoIterator<Item = &'a T>,
    T: Archive,
{
    type Archived = ArchivedIter<'static, T::Archived>;
}

// ────────────────────────────────────────────────────────────────────────────
//  BlockIndex – sparse index for O(1) random access
// ────────────────────────────────────────────────────────────────────────────

/// Sparse block index for fast random access into archived sequences.
///
/// Every `chunk_size` elements form a block.  The index stores the byte
/// offset of the first element marker in each block (relative to the
/// sequence `start_pos`).  To access element *i*, locate block
/// `i / chunk_size` in O(1) and scan at most `chunk_size` elements inside
/// that block.
#[derive(Clone)]
pub struct BlockIndex {
    chunk_size: usize,
    #[cfg(not(feature = "alloc"))]
    num_blocks: usize,
    /// Decoded absolute byte offsets for each block, relative to sequence
    /// `start_pos`.
    ///
    /// Under `no_alloc`, this is replaced by raw byte-range fields so
    /// offsets can be decoded on the fly.
    #[cfg(feature = "alloc")]
    offsets: Vec<usize>,
    #[cfg(not(feature = "alloc"))]
    raw_delta_start: usize,
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
            match super::varint::decode_u64::<usize, _>(&mut cursor, &mut ctx) {
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
    bytes: &'a [u8],
    start_pos: usize,
    len: usize,
    block_index: Option<BlockIndex>,
    _marker: PhantomData<A>,
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
fn decode_block_index<C>(
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
    let chunk_size: usize = super::varint::decode_u64(cursor, context)?;
    if chunk_size == 0 {
        return Err(DecodeError::ValidationError {
            message: "Block index chunk_size must be > 0",
            pos: cursor.pos(),
        });
    }

    // num_blocks (varint)
    let num_blocks: usize = super::varint::decode_u64(cursor, context)?;
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
            let delta: usize = super::varint::decode_u64(cursor, context)?;
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
            let _delta: usize = super::varint::decode_u64(cursor, context)?;
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

    let _chunk_size: usize = super::varint::decode_u64(cursor, context)?;
    let num_blocks: usize = super::varint::decode_u64(cursor, context)?;
    for _ in 0..num_blocks {
        let _delta: usize = super::varint::decode_u64(cursor, context)?;
    }
    Ok(())
}

/// Lazy decoding iterator over the elements of an `ArchivedIter`.
pub struct ArchivedIterIter<'a, A: Decode<'a>> {
    cursor: Cursor<'a>,
    remaining: usize,
    _marker: PhantomData<A>,
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

#[cfg(feature = "alloc")]
fn decode_next_element<'a, T: Decode<'a>>(cursor: &mut Cursor<'a>) -> Result<T::View, ZebinError> {
    let mut context = DummyContext;
    let marker = cursor.read_u8(&mut context)?;
    if marker != 1 {
        return Err(ZebinError::Decode(DecodeError::ValidationError {
            message: "Invalid sequence marker",
            pos: cursor.pos() - 1,
        }));
    }
    if <T as ArchivedLayout>::FIXED_SIZE.is_some() {
        cursor.align(<T as ArchivedLayout>::ALIGNMENT, &mut context)?;
    }
    Ok(T::decode(cursor, &mut context)?)
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<Vec<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
            out.push(view.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<VecDeque<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut out = VecDeque::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
            out.push_back(view.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<BTreeSet<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
    U: Ord,
{
    fn restore(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut out = BTreeSet::new();
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
            out.insert(view.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<BinaryHeap<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
    U: Ord,
{
    fn restore(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut out = BinaryHeap::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
            out.push(view.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<T, U> Restore<HashSet<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
    U: Eq + core::hash::Hash,
{
    fn restore(&self) -> Result<HashSet<U>, ZebinError> {
        let mut out = HashSet::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
            out.insert(view.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, I, U> Restore<IterArchive<I, U>> for ArchivedIter<'_, T>
where
    Self: Restore<I>,
{
    fn restore(&self) -> Result<IterArchive<I, U>, ZebinError> {
        Ok(IterArchive::new(self.restore()?))
    }
}

#[cfg(feature = "alloc")]
impl<T, K, V, UK, UV> Restore<BTreeMap<UK, UV>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a, View = (K, V)>,
    K: Restore<UK>,
    V: Restore<UV>,
    UK: Ord,
{
    fn restore(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let (k, v) = decode_next_element::<T>(&mut cursor)?;
            map.insert(k.restore()?, v.restore()?);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
impl<T, K, V, UK, UV> Restore<HashMap<UK, UV>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a, View = (K, V)>,
    K: Restore<UK>,
    V: Restore<UV>,
    UK: Eq + core::hash::Hash,
{
    fn restore(&self) -> Result<HashMap<UK, UV>, ZebinError> {
        let mut map = HashMap::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let (k, v) = decode_next_element::<T>(&mut cursor)?;
            map.insert(k.restore()?, v.restore()?);
        }
        Ok(map)
    }
}

/// Per-element resumable encoder for an owned-element sequence.
///
/// The element is moved into the encoder via `input(item)` and dropped after
/// the inner encoder finishes. This is the building block that makes streaming
/// owned-collection encoding (e.g. `Vec::into_iter`) actually release memory.
///
/// The element encoder is boxed when the `alloc` feature is enabled, which
/// breaks recursive type cycles for self-referential structs (`Node` ->
/// `Vec<Node>` -> `SeqEncoder<Node>` -> `Node::Encoder` -> ...).
struct SeqItemEncoder<'a, T: Encode + Archive + 'a> {
    #[cfg(feature = "alloc")]
    inner: Option<alloc::boxed::Box<<T as Encode>::Encoder<'a>>>,
    #[cfg(not(feature = "alloc"))]
    inner: Option<<T as Encode>::Encoder<'a>>,
}

impl<'a, T: Encode + Archive + 'a> SeqItemEncoder<'a, T> {
    fn new() -> Self {
        Self { inner: None }
    }

    fn take(&mut self) -> Self {
        Self {
            inner: self.inner.take(),
        }
    }

    fn get_or_insert_with<F>(&mut self, f: F) -> &mut <T as Encode>::Encoder<'a>
    where
        F: FnOnce() -> <T as Encode>::Encoder<'a>,
    {
        #[cfg(feature = "alloc")]
        {
            self.inner
                .get_or_insert_with(|| alloc::boxed::Box::new(f()))
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.inner.get_or_insert_with(f)
        }
    }

    fn as_mut(&mut self) -> Option<&mut <T as Encode>::Encoder<'a>> {
        #[cfg(feature = "alloc")]
        {
            self.inner.as_deref_mut()
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.inner.as_mut()
        }
    }

    fn finish<S: ByteSink + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        if let Some(encoder) = self.inner {
            encoder.finish(sink)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

/// Per-element resumable encoder for an owned-element sequence.
///
/// The element is moved into the encoder via `input(item)` and dropped after
/// the inner encoder finishes. This is the building block that makes streaming
/// owned-collection encoding (e.g. `Vec::into_iter`) actually release memory.
///
/// The element encoder is boxed when the `alloc` feature is enabled, which
/// breaks recursive type cycles for self-referential structs (`Node` ->
/// `Vec<Node>` -> `SeqEncoder<Node>` -> `Node::Encoder` -> ...).
pub struct SeqEncoder<'a, T: Encode + Archive + 'a> {
    next_item: Option<T>,
    marker: [u8; 1],
    marker_cursor: usize,
    aligned: bool,
    item_encoder: SeqItemEncoder<'a, T>,
    has_active_encoder: bool,
    encoder_started: bool,
    finished: bool,
    // ── Block index tracking ────────────────────────────────────────────
    enable_block_index: bool,
    element_count: usize,
    start_pos: Option<usize>,
    #[cfg(feature = "alloc")]
    block_offsets: Vec<usize>,
    // ── Block index write state (after sentinel) ────────────────────────
    #[cfg(feature = "alloc")]
    index_buf: Vec<u8>,
    #[cfg(feature = "alloc")]
    index_buf_cursor: usize,
}

impl<'a, T: Encode + Archive + 'a> SeqEncoder<'a, T> {
    pub fn new() -> Self {
        Self::with_index(false)
    }

    pub fn new_indexed() -> Self {
        Self::with_index(true)
    }

    fn with_index(enable: bool) -> Self {
        Self {
            next_item: None,
            marker: [0],
            marker_cursor: 1,
            aligned: false,
            item_encoder: SeqItemEncoder::new(),
            has_active_encoder: false,
            encoder_started: false,
            finished: false,
            enable_block_index: enable,
            element_count: 0,
            start_pos: None,
            #[cfg(feature = "alloc")]
            block_offsets: Vec::new(),
            #[cfg(feature = "alloc")]
            index_buf: Vec::new(),
            #[cfg(feature = "alloc")]
            index_buf_cursor: 0,
        }
    }
}

impl<'a, T: Encode + Archive + 'a> SeqEncoder<'a, T>
where
    T::Archived: ArchivedLayout,
{
    #[inline]
    fn try_align<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<bool, ZebinError> {
        if <T::Archived as ArchivedLayout>::FIXED_SIZE.is_none() || self.aligned {
            return Ok(true);
        }
        if sink
            .align(<T::Archived as ArchivedLayout>::ALIGNMENT)?
            .is_complete()
        {
            self.aligned = true;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl<'a, T: Encode + Archive + 'a> Default for SeqEncoder<'a, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T: Encode + Archive + 'a> SeqEncoder<'a, T>
where
    T::Archived: ArchivedLayout,
    T: Encode<Input<'a> = T>,
{
    pub fn is_finished(&self) -> bool {
        self.finished && self.marker_cursor == 1 && self.index_write_done()
    }

    /// Check whether the trailing index buffer has been fully flushed.
    #[inline]
    fn index_write_done(&self) -> bool {
        #[cfg(feature = "alloc")]
        {
            self.index_buf_cursor >= self.index_buf.len()
        }
        #[cfg(not(feature = "alloc"))]
        {
            true
        }
    }

    pub fn finish_ref<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if !self.finished {
            if self.next_item.is_some() || self.has_active_encoder || self.marker_cursor < 1 {
                return Err(ZebinError::SerializationError {
                    pos: sink.pos(),
                    message: "Encoder is busy",
                });
            }
            self.marker = [0];
            self.marker_cursor = 0;
            self.finished = true;

            // Build the block index buffer (only when enabled, alloc available,
            // and the sequence has more than chunk_size elements).
            #[cfg(feature = "alloc")]
            {
                if self.enable_block_index && self.element_count > DEFAULT_CHUNK_SIZE {
                    self.build_index_buf();
                }
            }
        }
        self.poll_pending(sink)
    }

    /// Serialize the block index section into `self.index_buf`.
    #[cfg(feature = "alloc")]
    fn build_index_buf(&mut self) {
        use super::varint::{encode_u64, encoded_len_u64};

        let chunk_size = DEFAULT_CHUNK_SIZE;
        let num_blocks = self.block_offsets.len();
        if num_blocks == 0 {
            return;
        }

        // Estimate capacity: magic(1) + chunk_size varint + num_blocks varint
        // + num_blocks * avg_delta_varint.
        let mut buf = Vec::with_capacity(2 + 2 + num_blocks * 4);

        // Magic byte
        buf.push(BLOCK_INDEX_MAGIC);

        // chunk_size (varint)
        {
            let len = encoded_len_u64(chunk_size as u64);
            let start = buf.len();
            buf.resize(start + len, 0);
            encode_u64(chunk_size as u64, &mut buf[start..]);
        }

        // num_blocks (varint)
        {
            let len = encoded_len_u64(num_blocks as u64);
            let start = buf.len();
            buf.resize(start + len, 0);
            encode_u64(num_blocks as u64, &mut buf[start..]);
        }

        // Delta-encoded offsets
        let mut prev = 0usize;
        for &offset in &self.block_offsets {
            let delta = offset - prev;
            prev = offset;
            let len = encoded_len_u64(delta as u64);
            let start = buf.len();
            buf.resize(start + len, 0);
            encode_u64(delta as u64, &mut buf[start..]);
        }

        self.index_buf = buf;
        self.index_buf_cursor = 0;
    }
}

impl<'a, T: Encode + Archive + 'a> Encoder for SeqEncoder<'a, T>
where
    T::Archived: ArchivedLayout,
    T: Encode<Input<'a> = T>,
{
    type Input = T;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if self.finished {
            return Err(ZebinError::SerializationError {
                pos: sink.pos(),
                message: "Encoder already finished",
            });
        }
        if self.next_item.is_some() || self.has_active_encoder || self.marker_cursor < 1 {
            return Err(ZebinError::SerializationError {
                pos: sink.pos(),
                message: "Encoder is busy",
            });
        }

        // ── Block index: record start_pos and block boundaries ──────────
        if self.enable_block_index {
            if self.element_count == 0 {
                self.start_pos = Some(sink.pos());
            }

            #[cfg(feature = "alloc")]
            {
                if self.element_count.is_multiple_of(DEFAULT_CHUNK_SIZE) {
                    let offset = sink.pos() - self.start_pos.expect("start_pos must be set");
                    self.block_offsets.push(offset);
                }
            }

            self.element_count += 1;
        }

        self.next_item = Some(item);
        self.marker = [1];
        self.marker_cursor = 0;
        self.aligned = false;

        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        loop {
            // ── Phase 1: flush the 1-byte sequence marker ──────────────────
            if self.marker_cursor < 1 {
                let remaining = 1 - self.marker_cursor;
                if sink
                    .write(&self.marker[self.marker_cursor..])?
                    .advance_cursor(&mut self.marker_cursor, remaining)
                    .is_pending()
                {
                    return Ok(Poll::Pending);
                }
            }

            // ── Phase 2: terminator byte written → flush block index ───────
            if self.finished && self.marker_cursor == 1 {
                // Flush block index buffer if present.
                #[cfg(feature = "alloc")]
                {
                    if self.index_buf_cursor < self.index_buf.len() {
                        let remaining = self.index_buf.len() - self.index_buf_cursor;
                        if sink
                            .write(&self.index_buf[self.index_buf_cursor..])?
                            .advance_cursor(&mut self.index_buf_cursor, remaining)
                            .is_pending()
                        {
                            return Ok(Poll::Pending);
                        }
                    }
                }
                return Ok(Poll::Ready(()));
            }

            // ── Phase 3: resume / complete an in-progress element encoder ──
            if self.has_active_encoder {
                // Shared alignment gate used by both the active-encoder and
                // the new-item branches.  Extracted here so the logic lives
                // in exactly one place.
                if !self.try_align(sink)? {
                    return Ok(Poll::Pending);
                }

                if self.encoder_started {
                    let encoder = self.item_encoder.as_mut().expect("active encoder missing");
                    match encoder.poll_pending(sink)? {
                        Poll::Pending => return Ok(Poll::Pending),
                        Poll::Ready(()) => {}
                    }
                }

                // Element fully encoded. Replace the inner encoder with None
                // so state from this element doesn't leak into the
                // next, and run its `finish` to flush any trailing padding.
                let completed = self.item_encoder.take();
                let _ = completed.finish(sink)?;
                self.has_active_encoder = false;
                self.encoder_started = false;
                self.aligned = false;
            }

            // ── Phase 4: start encoding the next queued item ───────────────
            if let Some(item) = self.next_item.take() {
                // Same alignment gate as Phase 3.
                if !self.try_align(sink)? {
                    self.next_item = Some(item);
                    return Ok(Poll::Pending);
                }

                let encoder = self.item_encoder.get_or_insert_with(T::encoder);
                match encoder.input(item, sink)? {
                    Poll::Pending => {
                        self.has_active_encoder = true;
                        self.encoder_started = true;
                        return Ok(Poll::Pending);
                    }
                    Poll::Ready(()) => {
                        self.has_active_encoder = true;
                        self.encoder_started = false;
                    }
                }
                continue;
            }

            if !self.finished {
                return Ok(Poll::Ready(()));
            }
        }
    }

    fn finish<S: ByteSink + ?Sized>(mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        let _ = self.finish_ref(sink)?;
        self.item_encoder.finish(sink)
    }
}

/// Owned-iterator sequence encoder: drains `S: IntoIterator<Item = T>` and
/// drops each element after encoding. This is the path that delivers the
/// "encode and drop" memory benefit for `Vec`, `BTreeMap`, etc.
pub struct OwnedIterEncoder<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Encode + Archive + 'a,
{
    iter: Option<S::IntoIter>,
    seq_encoder: SeqEncoder<'a, T>,
}

impl<'a, S, T> OwnedIterEncoder<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Encode + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            iter: None,
            seq_encoder: SeqEncoder::new(),
        }
    }

    /// Create an encoder that writes a trailing block index for O(1)
    /// random access during decode.
    pub fn new_indexed() -> Self {
        Self {
            iter: None,
            seq_encoder: SeqEncoder::new_indexed(),
        }
    }
}

impl<'a, S, T> Default for OwnedIterEncoder<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Encode + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, S, T> Encoder for OwnedIterEncoder<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Encode<Input<'a> = T> + Archive + 'a,
    T::Archived: ArchivedLayout,
{
    type Input = S;

    fn input<Sink: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        self.iter = Some(item.into_iter());
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: ByteSink + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        let iter = self.iter.as_mut().ok_or(ZebinError::SerializationError {
            pos: sink.pos(),
            message: "OwnedIterEncoder polled before input",
        })?;
        loop {
            if self.seq_encoder.poll_pending(sink)?.is_pending() {
                return Ok(Poll::Pending);
            }

            if self.seq_encoder.is_finished() {
                return Ok(Poll::Ready(()));
            }

            if !self.seq_encoder.finished {
                if let Some(item) = iter.next() {
                    if self.seq_encoder.input(item, sink)?.is_pending() {
                        return Ok(Poll::Pending);
                    }
                } else {
                    if self.seq_encoder.finish_ref(sink)?.is_pending() {
                        return Ok(Poll::Pending);
                    }
                }
            }
        }
    }

    fn finish<Sink: ByteSink + ?Sized>(self, sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        self.seq_encoder.finish(sink)
    }
}

#[cfg(feature = "alloc")]
impl<I, T> Encode for IterArchive<I, T>
where
    I: IntoIterator<Item = T>,
    for<'a> &'a I: IntoIterator<Item = &'a T>,
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
    for<'a> T: Encode<Input<'a> = T> + 'a,
{
    type Input<'a>
        = IterArchive<I, T>
    where
        Self: 'a;
    type Encoder<'a>
        = OwnedIterEncoder<'a, IterArchive<I, T>, T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        OwnedIterEncoder::new_indexed()
    }
}

impl<I, T> MeasureBody for IterArchive<I, T>
where
    for<'a> &'a I: IntoIterator<Item = &'a T>,
    T: MeasureBody + Archive,
    T::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let mut pos = 0usize;
        let alignment = <T::Archived as ArchivedLayout>::ALIGNMENT.get();
        let fixed = <T::Archived as ArchivedLayout>::FIXED_SIZE.is_some();
        let mut count = 0usize;
        for item in (&self.0).into_iter() {
            // 1-byte sequence marker
            pos = pos
                .checked_add(1)
                .ok_or(ZebinError::ArithmeticOverflow { pos })?;
            if fixed {
                let pad = (alignment - (pos % alignment)) % alignment;
                pos = pos
                    .checked_add(pad)
                    .ok_or(ZebinError::ArithmeticOverflow { pos })?;
            }
            pos = pos
                .checked_add(item.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos })?;
            count += 1;
        }
        // Trailing 0x00 terminator byte
        pos = pos
            .checked_add(1)
            .ok_or(ZebinError::ArithmeticOverflow { pos })?;

        // Block index overhead (only when count > DEFAULT_CHUNK_SIZE)
        if count > DEFAULT_CHUNK_SIZE {
            pos = pos
                .checked_add(measure_block_index_overhead(count, pos)?)
                .ok_or(ZebinError::ArithmeticOverflow { pos })?;
        }

        Ok(pos)
    }
}

/// Estimate the byte overhead of the block index section.
///
/// `seq_body_len` is the byte length of elements + sentinel (used to
/// estimate varint sizes for delta offsets – the actual deltas aren't
/// known here, so we use a conservative upper bound based on `seq_body_len`).
pub fn measure_block_index_overhead(
    element_count: usize,
    seq_body_len: usize,
) -> Result<usize, ZebinError> {
    use super::varint::encoded_len_u64;

    let chunk_size = DEFAULT_CHUNK_SIZE;
    let num_blocks = element_count.div_ceil(chunk_size);

    let mut overhead = 0usize;
    // magic byte
    overhead += 1;
    // chunk_size varint
    overhead += encoded_len_u64(chunk_size as u64);
    // num_blocks varint
    overhead += encoded_len_u64(num_blocks as u64);
    // Each delta offset varint: upper bound is the full sequence length.
    // The average delta is seq_body_len / num_blocks, use the max for safety.
    let max_delta = seq_body_len;
    let delta_varint_len = encoded_len_u64(max_delta as u64);
    overhead += num_blocks * delta_varint_len;

    Ok(overhead)
}

// Backwards-compatible alias so external uses still resolve.
pub type IterEncoder<'a, S, T> = OwnedIterEncoder<'a, S, T>;
