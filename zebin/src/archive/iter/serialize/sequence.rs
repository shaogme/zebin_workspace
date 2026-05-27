use core::task::Poll;

use crate::{error::ZebinError, prelude::*};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use super::SeqItemSerializer;
#[cfg(feature = "alloc")]
use crate::archive_impl::iter::{BLOCK_INDEX_MAGIC, DEFAULT_CHUNK_SIZE};

/// Per-element resumable serializer for an owned-element sequence.
///
/// The element is moved into the serializer via `input(item)` and dropped after
/// the inner serializer finishes. This is the building block that makes streaming
/// owned-collection encoding (e.g. `Vec::into_iter`) actually release memory.
///
/// The element serializer is boxed when the `alloc` feature is enabled, which
/// breaks recursive type cycles for self-referential structs (`Node` ->
/// `Vec<Node>` -> `SeqSerializer<Node>` -> `Node::Serializer` -> ...).
#[cfg(feature = "alloc")]
pub struct SeqSerializerAllocState {
    pub(crate) block_offsets: Vec<usize>,
    pub(crate) index_buf: Vec<u8>,
    pub(crate) index_buf_cursor: usize,
}

pub struct SeqSerializer<'a, T: Serialize + Archive + 'a> {
    pub(crate) next_item: Option<T>,
    pub(crate) marker: [u8; 1],
    pub(crate) marker_cursor: usize,
    pub(crate) aligned: bool,
    pub(crate) item_serializer: SeqItemSerializer<'a, T>,
    pub(crate) has_active_serializer: bool,
    pub(crate) serializer_started: bool,
    pub(crate) finished: bool,
    // ── Block index tracking ────────────────────────────────────────────
    pub(crate) enable_block_index: bool,
    pub(crate) element_count: usize,
    pub(crate) start_pos: Option<usize>,
    #[cfg(feature = "alloc")]
    pub(crate) alloc_state: SeqSerializerAllocState,
}

impl<'a, T: Serialize + Archive + 'a> SeqSerializer<'a, T> {
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
            item_serializer: SeqItemSerializer::new(),
            has_active_serializer: false,
            serializer_started: false,
            finished: false,
            enable_block_index: enable,
            element_count: 0,
            start_pos: None,
            #[cfg(feature = "alloc")]
            alloc_state: SeqSerializerAllocState {
                block_offsets: Vec::new(),
                index_buf: Vec::new(),
                index_buf_cursor: 0,
            },
        }
    }
}

impl<'a, T: Serialize + Archive + 'a> SeqSerializer<'a, T>
where
    T::Archived: ArchivedLayout,
{
    #[inline]
    fn try_align<S: StorageMut + ?Sized>(&mut self, sink: &mut S) -> Result<bool, ZebinError> {
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

impl<'a, T: Serialize + Archive + 'a> Default for SeqSerializer<'a, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T: Serialize + Archive + 'a> SeqSerializer<'a, T>
where
    T::Archived: ArchivedLayout,
    T: Serialize<Input<'a> = T>,
{
    pub fn is_finished(&self) -> bool {
        self.finished && self.marker_cursor == 1 && self.index_write_done()
    }

    /// Check whether the trailing index buffer has been fully flushed.
    #[inline]
    fn index_write_done(&self) -> bool {
        #[cfg(feature = "alloc")]
        {
            self.alloc_state.index_buf_cursor >= self.alloc_state.index_buf.len()
        }
        #[cfg(not(feature = "alloc"))]
        {
            true
        }
    }

    pub fn finish_ref<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if !self.finished {
            if self.next_item.is_some() || self.has_active_serializer || self.marker_cursor < 1 {
                return Err(ZebinError::SerializationError {
                    pos: sink.pos(),
                    message: "Serializer is busy",
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
        use crate::archive_impl::varint::{serialize_u64, serialized_len_u64};

        let chunk_size = DEFAULT_CHUNK_SIZE;
        let num_blocks = self.alloc_state.block_offsets.len();
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
            let len = serialized_len_u64(chunk_size as u64);
            let start = buf.len();
            buf.resize(start + len, 0);
            serialize_u64(chunk_size as u64, &mut buf[start..]);
        }

        // num_blocks (varint)
        {
            let len = serialized_len_u64(num_blocks as u64);
            let start = buf.len();
            buf.resize(start + len, 0);
            serialize_u64(num_blocks as u64, &mut buf[start..]);
        }

        // Delta-serialized offsets
        let mut prev = 0usize;
        for &offset in &self.alloc_state.block_offsets {
            let delta = offset - prev;
            prev = offset;
            let len = serialized_len_u64(delta as u64);
            let start = buf.len();
            buf.resize(start + len, 0);
            serialize_u64(delta as u64, &mut buf[start..]);
        }

        self.alloc_state.index_buf = buf;
        self.alloc_state.index_buf_cursor = 0;
    }
}

impl<'a, T: Serialize + Archive + 'a> Serializer for SeqSerializer<'a, T>
where
    T::Archived: ArchivedLayout,
    T: Serialize<Input<'a> = T>,
{
    type Input = T;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if self.finished {
            return Err(ZebinError::SerializationError {
                pos: sink.pos(),
                message: "Serializer already finished",
            });
        }
        if self.next_item.is_some() || self.has_active_serializer || self.marker_cursor < 1 {
            return Err(ZebinError::SerializationError {
                pos: sink.pos(),
                message: "Serializer is busy",
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
                    self.alloc_state.block_offsets.push(offset);
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

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
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
                    if self.alloc_state.index_buf_cursor < self.alloc_state.index_buf.len() {
                        let remaining =
                            self.alloc_state.index_buf.len() - self.alloc_state.index_buf_cursor;
                        if sink
                            .write(
                                &self.alloc_state.index_buf[self.alloc_state.index_buf_cursor..],
                            )?
                            .advance_cursor(&mut self.alloc_state.index_buf_cursor, remaining)
                            .is_pending()
                        {
                            return Ok(Poll::Pending);
                        }
                    }
                }
                return Ok(Poll::Ready(()));
            }

            // ── Phase 3: resume / complete an in-progress element serializer ──
            if self.has_active_serializer {
                // Shared alignment gate used by both the active-serializer and
                // the new-item branches.  Extracted here so the logic lives
                // in exactly one place.
                if !self.try_align(sink)? {
                    return Ok(Poll::Pending);
                }

                if self.serializer_started {
                    let serializer = self
                        .item_serializer
                        .as_mut()
                        .expect("active serializer missing");
                    match serializer.poll_pending(sink)? {
                        Poll::Pending => return Ok(Poll::Pending),
                        Poll::Ready(()) => {}
                    }
                }

                // Element fully serialized. Replace the inner serializer with None
                // so state from this element doesn't leak into the
                // next, and run its `finish` to flush any trailing padding.
                let completed = self.item_serializer.take();
                let _ = completed.finish(sink)?;
                self.has_active_serializer = false;
                self.serializer_started = false;
                self.aligned = false;
            }

            // ── Phase 4: start encoding the next queued item ───────────────
            if let Some(item) = self.next_item.take() {
                // Same alignment gate as Phase 3.
                if !self.try_align(sink)? {
                    self.next_item = Some(item);
                    return Ok(Poll::Pending);
                }

                let serializer = self.item_serializer.get_or_insert_with(T::serializer);
                match serializer.input(item, sink)? {
                    Poll::Pending => {
                        self.has_active_serializer = true;
                        self.serializer_started = true;
                        return Ok(Poll::Pending);
                    }
                    Poll::Ready(()) => {
                        self.has_active_serializer = true;
                        self.serializer_started = false;
                    }
                }
                continue;
            }

            if !self.finished {
                return Ok(Poll::Ready(()));
            }
        }
    }

    fn finish<S: StorageMut + ?Sized>(mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        let _ = self.finish_ref(sink)?;
        self.item_serializer.finish(sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::SliceSerializer;

    #[test]
    fn test_seq_serializer_busy_errors() {
        let mut buf = [0u8; 1];
        let mut sink = SliceSerializer::new(&mut buf, 0);
        let mut serializer: SeqSerializer<'_, u32> = SeqSerializer::new();

        // Feed first item
        let poll_res = serializer.input(42, &mut sink);
        assert!(poll_res.is_ok());

        // Try feeding again while busy
        let poll_err = serializer.input(43, &mut sink);
        assert!(poll_err.is_err());
    }

    #[test]
    fn test_seq_serializer_finished_error() {
        let mut buf = [0u8; 100];
        let mut sink = SliceSerializer::new(&mut buf, 0);
        let mut serializer: SeqSerializer<'_, u32> = SeqSerializer::new();

        let _ = serializer.finish_ref(&mut sink);
        let res = serializer.input(42, &mut sink);
        assert!(res.is_err());
    }
}
