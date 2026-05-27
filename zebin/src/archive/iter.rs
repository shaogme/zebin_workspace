use core::marker::PhantomData;
use core::num::NonZeroUsize;

use crate::{
    prelude::*,
    validation::{ValidationContext, ValidationPathSegment},
};

// Declare submodules
#[path = "iter/decode.rs"]
mod decode;
#[path = "iter/encode.rs"]
mod encode;
#[path = "iter/restore.rs"]
mod restore;

// Re-export public API
pub use decode::ArchivedIter;
#[cfg(feature = "alloc")]
pub(crate) use decode::skip_block_index;
pub(crate) use encode::OwnedIterEncoder;
pub use encode::{IterEncoder, SeqEncoder, measure_block_index_overhead};

pub(crate) struct DummyContext;

impl ValidationContext for DummyContext {
    fn push_depth(&mut self) -> Result<(), AccessError> {
        Ok(())
    }

    fn pop_depth(&mut self) {}

    fn push_path(&mut self, _segment: ValidationPathSegment) {}

    fn pop_path(&mut self) {}

    fn record_error_path(&mut self) {}

    fn check_range(&mut self, _pos: usize, _size: usize) -> Result<(), AccessError> {
        Ok(())
    }

    fn check_alignment(
        &mut self,
        _pos: usize,
        _alignment: NonZeroUsize,
    ) -> Result<(), AccessError> {
        Ok(())
    }

    fn check_sequence_len(&mut self, _len: usize, _pos: usize) -> Result<(), AccessError> {
        Ok(())
    }
}

/// Maximum number of elements accepted in a single sequence during
/// decoding.  Prevents maliciously crafted input from triggering
/// integer-overflow or denial-of-service via extremely large `len` fields.
pub(crate) const MAX_SEQUENCE_LEN: usize = 1 << 28; // 256 Mi elements

/// Default number of elements per block in the sparse index.
pub(crate) const DEFAULT_CHUNK_SIZE: usize = 64;

/// Magic byte appended after the block index section to signal its presence.
/// Chosen to not conflict with the sequence marker (0x01) or sentinel (0x00).
pub(crate) const BLOCK_INDEX_MAGIC: u8 = 0x42;

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
        #[cfg(feature = "alloc")]
        {
            OwnedIterEncoder::new_indexed()
        }
        #[cfg(not(feature = "alloc"))]
        {
            OwnedIterEncoder::new()
        }
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
