use core::task::Poll;

use crate::{error::ZebinError, prelude::*};

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

use super::DEFAULT_CHUNK_SIZE;

/// Per-element resumable encoder for an owned-element sequence.
///
/// The element is moved into the encoder via `input(item)` and dropped after
/// the inner encoder finishes. This is the building block that makes streaming
/// owned-collection encoding (e.g. `Vec::into_iter`) actually release memory.
///
/// The element encoder is boxed when the `alloc` feature is enabled, which
/// breaks recursive type cycles for self-referential structs (`Node` ->
/// `Vec<Node>` -> `SeqEncoder<Node>` -> `Node::Encoder` -> ...).
pub(crate) struct SeqItemEncoder<'a, T: Encode + Archive + 'a> {
    #[cfg(feature = "alloc")]
    pub(crate) inner: Option<Box<<T as Encode>::Encoder<'a>>>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) inner: Option<<T as Encode>::Encoder<'a>>,
}

impl<'a, T: Encode + Archive + 'a> SeqItemEncoder<'a, T> {
    pub(crate) fn new() -> Self {
        Self { inner: None }
    }

    pub(crate) fn take(&mut self) -> Self {
        Self {
            inner: self.inner.take(),
        }
    }

    pub(crate) fn get_or_insert_with<F>(&mut self, f: F) -> &mut <T as Encode>::Encoder<'a>
    where
        F: FnOnce() -> <T as Encode>::Encoder<'a>,
    {
        #[cfg(feature = "alloc")]
        {
            self.inner.get_or_insert_with(|| Box::new(f()))
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.inner.get_or_insert_with(f)
        }
    }

    pub(crate) fn as_mut(&mut self) -> Option<&mut <T as Encode>::Encoder<'a>> {
        #[cfg(feature = "alloc")]
        {
            self.inner.as_deref_mut()
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.inner.as_mut()
        }
    }

    pub(crate) fn finish<S: StorageMut + ?Sized>(
        self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
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
#[path = "encode/sequence.rs"]
mod sequence;
pub use sequence::SeqEncoder;

/// Owned-iterator sequence encoder: drains `S: IntoIterator<Item = T>` and
/// drops each element after encoding. This is the path that delivers the
/// "encode and drop" memory benefit for `Vec`, `BTreeMap`, etc.
pub struct OwnedIterEncoder<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Encode + Archive + 'a,
{
    pub(crate) iter: Option<S::IntoIter>,
    pub(crate) seq_encoder: SeqEncoder<'a, T>,
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
    /// random access during deserialize.
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

    fn input<Sink: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        self.iter = Some(item.into_iter());
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: StorageMut + ?Sized>(
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

    fn finish<Sink: StorageMut + ?Sized>(self, sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        self.seq_encoder.finish(sink)
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
    use super::super::varint::encoded_len_u64;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::SliceEncoder;

    #[test]
    fn test_measure_block_index_overhead() {
        // Just verify it computes something positive and runs without overflow.
        let overhead = measure_block_index_overhead(100, 500).unwrap();
        assert!(overhead > 0);
    }

    #[test]
    fn test_owned_iter_encoder_polled_before_input() {
        let mut buf = [0u8; 100];
        let mut sink = SliceEncoder::new(&mut buf, 0);
        let mut encoder: OwnedIterEncoder<'_, Vec<u32>, u32> = OwnedIterEncoder::new();

        let res = encoder.poll_pending(&mut sink);
        assert!(res.is_err());
    }
}
