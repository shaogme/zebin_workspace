use core::task::Poll;

use crate::{error::ZebinError, prelude::*};

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

use super::DEFAULT_CHUNK_SIZE;

/// Per-element resumable serializer for an owned-element sequence.
///
/// The element is moved into the serializer via `input(item)` and dropped after
/// the inner serializer finishes. This is the building block that makes streaming
/// owned-collection encoding (e.g. `Vec::into_iter`) actually release memory.
///
/// The element serializer is boxed when the `alloc` feature is enabled, which
/// breaks recursive type cycles for self-referential structs (`Node` ->
/// `Vec<Node>` -> `SeqSerializer<Node>` -> `Node::Serializer` -> ...).
pub(crate) struct SeqItemSerializer<'a, T: Serialize + Archive + 'a> {
    #[cfg(feature = "alloc")]
    pub(crate) inner: Option<Box<<T as Serialize>::Serializer<'a>>>,
    #[cfg(not(feature = "alloc"))]
    pub(crate) inner: Option<<T as Serialize>::Serializer<'a>>,
}

impl<'a, T: Serialize + Archive + 'a> SeqItemSerializer<'a, T> {
    pub(crate) fn new() -> Self {
        Self { inner: None }
    }

    pub(crate) fn take(&mut self) -> Self {
        Self {
            inner: self.inner.take(),
        }
    }

    pub(crate) fn get_or_insert_with<F>(&mut self, f: F) -> &mut <T as Serialize>::Serializer<'a>
    where
        F: FnOnce() -> <T as Serialize>::Serializer<'a>,
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

    pub(crate) fn as_mut(&mut self) -> Option<&mut <T as Serialize>::Serializer<'a>> {
        #[cfg(feature = "alloc")]
        {
            self.inner.as_deref_mut()
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.inner.as_mut()
        }
    }

    pub(crate) fn finish(self, sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        if let Some(serializer) = self.inner {
            serializer.finish(sink)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

/// Per-element resumable serializer for an owned-element sequence.
///
/// The element is moved into the serializer via `input(item)` and dropped after
/// the inner serializer finishes. This is the building block that makes streaming
/// owned-collection encoding (e.g. `Vec::into_iter`) actually release memory.
///
/// The element serializer is boxed when the `alloc` feature is enabled, which
/// breaks recursive type cycles for self-referential structs (`Node` ->
/// `Vec<Node>` -> `SeqSerializer<Node>` -> `Node::Serializer` -> ...).
#[path = "serialize/sequence.rs"]
mod sequence;
pub use sequence::SeqSerializer;

/// Owned-iterator sequence serializer: drains `S: IntoIterator<Item = T>` and
/// drops each element after encoding. This is the path that delivers the
/// "serialize and drop" memory benefit for `Vec`, `BTreeMap`, etc.
pub struct OwnedIterSerializer<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Serialize + Archive + 'a,
{
    pub(crate) iter: Option<S::IntoIter>,
    pub(crate) seq_serializer: SeqSerializer<'a, T>,
}

impl<'a, S, T> OwnedIterSerializer<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Serialize + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            iter: None,
            seq_serializer: SeqSerializer::new(),
        }
    }

    /// Create an serializer that writes a trailing block index for O(1)
    /// random access during deserialize.
    pub fn new_indexed() -> Self {
        Self {
            iter: None,
            seq_serializer: SeqSerializer::new_indexed(),
        }
    }
}

impl<'a, S, T> Default for OwnedIterSerializer<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Serialize + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, S, T> Serializer for OwnedIterSerializer<'a, S, T>
where
    S: IntoIterator<Item = T>,
    T: Serialize<Input<'a> = T> + Archive + 'a,
    T::Archived: ArchivedLayout,
{
    type Input = S;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut dyn CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        self.iter = Some(item.into_iter());
        self.poll_pending(sink)
    }

    fn poll_pending(&mut self, sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        let iter = self.iter.as_mut().ok_or(ZebinError::SerializeError {
            pos: sink.pos(),
            message: "OwnedIterSerializer polled before input",
        })?;
        loop {
            if self.seq_serializer.poll_pending(sink)?.is_pending() {
                return Ok(Poll::Pending);
            }

            if self.seq_serializer.is_finished() {
                return Ok(Poll::Ready(()));
            }

            if !self.seq_serializer.finished {
                if let Some(item) = iter.next() {
                    if self.seq_serializer.input(item, sink)?.is_pending() {
                        return Ok(Poll::Pending);
                    }
                } else {
                    if self.seq_serializer.finish_ref(sink)?.is_pending() {
                        return Ok(Poll::Pending);
                    }
                }
            }
        }
    }

    fn finish(self, sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.seq_serializer.finish(sink)
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
    use super::super::varint::serialized_len_u64;

    let chunk_size = DEFAULT_CHUNK_SIZE;
    let num_blocks = element_count.div_ceil(chunk_size);

    let mut overhead = 0usize;
    // magic byte
    overhead += 1;
    // chunk_size varint
    overhead += serialized_len_u64(chunk_size as u64);
    // num_blocks varint
    overhead += serialized_len_u64(num_blocks as u64);
    // Each delta offset varint: upper bound is the full sequence length.
    // The average delta is seq_body_len / num_blocks, use the max for safety.
    let max_delta = seq_body_len;
    let delta_varint_len = serialized_len_u64(max_delta as u64);
    overhead += num_blocks * delta_varint_len;

    Ok(overhead)
}

// Backwards-compatible alias so external uses still resolve.
pub type IterSerializer<'a, S, T> = OwnedIterSerializer<'a, S, T>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::SliceSerializer;

    #[test]
    fn test_measure_block_index_overhead() {
        // Just verify it computes something positive and runs without overflow.
        let overhead = measure_block_index_overhead(100, 500).unwrap();
        assert!(overhead > 0);
    }

    #[test]
    fn test_owned_iter_serializer_polled_before_input() {
        let mut buf = [0u8; 100];
        let mut sink = SliceSerializer::new(&mut buf, 0);
        let mut serializer: OwnedIterSerializer<'_, Vec<u32>, u32> = OwnedIterSerializer::new();

        let mut writer = sink.into_cursor_mut();
        let res = serializer.poll_pending(&mut writer);
        assert!(res.is_err());
    }
}
