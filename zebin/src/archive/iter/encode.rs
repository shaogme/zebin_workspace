use core::task::Poll;

use crate::{
    prelude::*,
    error::ZebinError,
};

#[cfg(feature = "alloc")]
use alloc::{vec::Vec, boxed::Box};

use super::{DEFAULT_CHUNK_SIZE, BLOCK_INDEX_MAGIC};

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
            self.inner
                .get_or_insert_with(|| Box::new(f()))
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

    pub(crate) fn finish<S: ByteSink + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
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
    pub(crate) next_item: Option<T>,
    pub(crate) marker: [u8; 1],
    pub(crate) marker_cursor: usize,
    pub(crate) aligned: bool,
    pub(crate) item_encoder: SeqItemEncoder<'a, T>,
    pub(crate) has_active_encoder: bool,
    pub(crate) encoder_started: bool,
    pub(crate) finished: bool,
    // ── Block index tracking ────────────────────────────────────────────
    pub(crate) enable_block_index: bool,
    pub(crate) element_count: usize,
    pub(crate) start_pos: Option<usize>,
    #[cfg(feature = "alloc")]
    pub(crate) block_offsets: Vec<usize>,
    // ── Block index write state (after sentinel) ────────────────────────
    #[cfg(feature = "alloc")]
    pub(crate) index_buf: Vec<u8>,
    #[cfg(feature = "alloc")]
    pub(crate) index_buf_cursor: usize,
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
        use super::super::varint::{encode_u64, encoded_len_u64};

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
