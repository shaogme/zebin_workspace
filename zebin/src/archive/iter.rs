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

/// Validation context used during lazy decoding.
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

/// The archived representation of an iterator-based collection.
/// Decodes in O(1) time without any memory allocation.
#[derive(Clone)]
pub struct ArchivedIter<'a, A> {
    bytes: &'a [u8],
    start_pos: usize,
    len: usize,
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
        let mut len = 0;
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
            if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                cursor.align(<A as ArchivedLayout>::ALIGNMENT, context)?;
            }
            let mut guard = context.push_index(len);
            A::validate(cursor, &mut *guard)?;
            len += 1;
        }
        Ok(ArchivedIter {
            bytes: cursor.bytes(),
            start_pos,
            len,
            _marker: PhantomData,
        })
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let mut len = 0;
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
            if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
                cursor.align(<A as ArchivedLayout>::ALIGNMENT, context)?;
            }
            let mut guard = context.push_index(len);
            A::validate(cursor, &mut *guard)?;
            len += 1;
        }
        Ok(())
    }
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
fn decode_next_element<'a, T: Decode<'a>>(
    cursor: &mut Cursor<'a>,
    context: &mut DummyContext,
) -> Result<T::View, ZebinError> {
    let marker = cursor.read_u8(context)?;
    if marker != 1 {
        return Err(ZebinError::Decode(DecodeError::ValidationError {
            message: "Invalid sequence marker",
            pos: cursor.pos() - 1,
        }));
    }
    if <T as ArchivedLayout>::FIXED_SIZE.is_some() {
        cursor.align(<T as ArchivedLayout>::ALIGNMENT, context)?;
    }
    Ok(T::decode(cursor, context)?)
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor, &mut context)?;
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor, &mut context)?;
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor, &mut context)?;
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor, &mut context)?;
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor, &mut context)?;
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let (k, v) = decode_next_element::<T>(&mut cursor, &mut context)?;
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
        let mut context = DummyContext;
        for _ in 0..self.len {
            let (k, v) = decode_next_element::<T>(&mut cursor, &mut context)?;
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
}

impl<'a, T: Encode + Archive + 'a> SeqEncoder<'a, T> {
    pub fn new() -> Self {
        Self {
            next_item: None,
            marker: [0],
            marker_cursor: 1,
            aligned: false,
            item_encoder: SeqItemEncoder::new(),
            has_active_encoder: false,
            encoder_started: false,
            finished: false,
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
        self.finished && self.marker_cursor == 1
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
        }
        self.poll_pending(sink)
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

        self.next_item = Some(item);
        self.marker = [1];
        self.marker_cursor = 0;
        self.aligned = false;

        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        loop {
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

            if self.finished && self.marker_cursor == 1 {
                return Ok(Poll::Ready(()));
            }

            if self.has_active_encoder {
                if <T::Archived as ArchivedLayout>::FIXED_SIZE.is_some() && !self.aligned {
                    if sink
                        .align(<T::Archived as ArchivedLayout>::ALIGNMENT)?
                        .is_complete()
                    {
                        self.aligned = true;
                    } else {
                        return Ok(Poll::Pending);
                    }
                }

                if self.encoder_started {
                    let encoder = self.item_encoder.as_mut().expect("active encoder missing");
                    let res = encoder.poll_pending(sink)?;

                    match res {
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

            if let Some(item) = self.next_item.take() {
                if <T::Archived as ArchivedLayout>::FIXED_SIZE.is_some() && !self.aligned {
                    if sink
                        .align(<T::Archived as ArchivedLayout>::ALIGNMENT)?
                        .is_complete()
                    {
                        self.aligned = true;
                    } else {
                        self.next_item = Some(item);
                        return Ok(Poll::Pending);
                    }
                }

                let encoder = self.item_encoder.get_or_insert_with(T::encoder);
                let res = encoder.input(item, sink)?;

                match res {
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
        OwnedIterEncoder::new()
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
        for item in (&self.0).into_iter() {
            pos = pos
                .checked_add(1)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            if fixed {
                let pad = (alignment - (pos % alignment)) % alignment;
                pos = pos
                    .checked_add(pad)
                    .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            }
            pos = pos
                .checked_add(item.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        pos = pos
            .checked_add(1)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        Ok(pos)
    }
}

// Backwards-compatible alias so external uses still resolve.
pub type IterEncoder<'a, S, T> = OwnedIterEncoder<'a, S, T>;
