use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::task::Poll;

use crate::{
    prelude::*,
    validation::{ValidationContext, ValidationPathSegment},
};

#[cfg(feature = "alloc")]
use alloc::{
    boxed::Box,
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

pub struct CurrentEncoder<'a, T: Encode + 'a> {
    #[cfg(feature = "alloc")]
    inner: Box<(<T as Encode>::Encoder<'a>, bool)>,
    #[cfg(not(feature = "alloc"))]
    inner: (<T as Encode>::Encoder<'a>, bool),
}

impl<'a, T: Encode + 'a> CurrentEncoder<'a, T> {
    pub fn new(encoder: <T as Encode>::Encoder<'a>, started: bool) -> Self {
        Self {
            #[cfg(feature = "alloc")]
            inner: Box::new((encoder, started)),
            #[cfg(not(feature = "alloc"))]
            inner: (encoder, started),
        }
    }

    pub fn get_mut(&mut self) -> (&mut <T as Encode>::Encoder<'a>, &mut bool) {
        #[cfg(feature = "alloc")]
        {
            let (ref mut encoder, ref mut started) = *self.inner;
            (encoder, started)
        }
        #[cfg(not(feature = "alloc"))]
        {
            let (ref mut encoder, ref mut started) = self.inner;
            (encoder, started)
        }
    }

    pub fn into_inner(self) -> (<T as Encode>::Encoder<'a>, bool) {
        #[cfg(feature = "alloc")]
        {
            *self.inner
        }
        #[cfg(not(feature = "alloc"))]
        {
            self.inner
        }
    }
}

pub struct SeqEncoder<'a, T: Encode + Archive + 'a> {
    next_item: Option<&'a T>,
    marker: [u8; 1],
    marker_cursor: usize,
    aligned: bool,
    current_encoder: Option<CurrentEncoder<'a, T>>,
    finished: bool,
}

impl<'a, T: Encode + Archive + 'a> SeqEncoder<'a, T> {
    pub fn new() -> Self {
        Self {
            next_item: None,
            marker: [0],
            marker_cursor: 1,
            aligned: false,
            current_encoder: None,
            finished: false,
        }
    }
}

impl<'a, T: Encode + Archive + 'a> SeqEncoder<'a, T>
where
    T::Archived: ArchivedLayout,
{
    pub fn is_finished(&self) -> bool {
        self.finished && self.marker_cursor == 1
    }

    pub fn finish_ref<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if !self.finished {
            if self.next_item.is_some() || self.current_encoder.is_some() || self.marker_cursor < 1
            {
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

impl<'a, T: Encode + Archive + 'a> Encoder<'a> for SeqEncoder<'a, T>
where
    T::Archived: ArchivedLayout,
{
    type Input = &'a T;

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
        if self.next_item.is_some() || self.current_encoder.is_some() || self.marker_cursor < 1 {
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

            if let Some(state) = &mut self.current_encoder {
                let (encoder, started) = state.get_mut();

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

                if *started {
                    match encoder.poll_pending(sink)? {
                        Poll::Pending => return Ok(Poll::Pending),
                        Poll::Ready(()) => {}
                    }
                }

                let (encoder, _) = self.current_encoder.take().unwrap().into_inner();
                let _ = encoder.finish(sink)?;
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

                let mut encoder = item.begin_encode()?;
                match encoder.input(item, sink)? {
                    Poll::Pending => {
                        self.current_encoder = Some(CurrentEncoder::new(encoder, true));
                        return Ok(Poll::Pending);
                    }
                    Poll::Ready(()) => {
                        self.current_encoder = Some(CurrentEncoder::new(encoder, false));
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
        self.finish_ref(sink)
    }
}

pub struct IterEncoder<'a, S: ?Sized, T, I: ?Sized = S>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + 'a,
{
    iter: <&'a S as IntoIterator>::IntoIter,
    seq_encoder: SeqEncoder<'a, T>,
    _phantom: PhantomData<&'a I>,
}

impl<'a, S: ?Sized, T, I: ?Sized> IterEncoder<'a, S, T, I>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + 'a,
{
    pub fn new(inner: &'a S) -> Result<Self, ZebinError> {
        let iter = inner.into_iter();
        Ok(Self {
            iter,
            seq_encoder: SeqEncoder::new(),
            _phantom: PhantomData,
        })
    }
}

impl<'a, S: ?Sized, T, I: ?Sized> Encoder<'a> for IterEncoder<'a, S, T, I>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + 'a,
    T::Archived: ArchivedLayout,
{
    type Input = &'a I;

    fn input<Sink: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: ByteSink + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<Poll<()>, ZebinError> {
        loop {
            if self.seq_encoder.poll_pending(sink)?.is_pending() {
                return Ok(Poll::Pending);
            }

            if self.seq_encoder.is_finished() {
                return Ok(Poll::Ready(()));
            }

            if !self.seq_encoder.finished {
                if let Some(item) = self.iter.next() {
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

    fn finish<Sink: ByteSink + ?Sized>(self, _sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<I, T> Encode for IterArchive<I, T>
where
    for<'a> &'a I: IntoIterator<Item = &'a T>,
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = IterEncoder<'a, I, T, IterArchive<I, T>>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        IterEncoder::new(&self.0)
    }
}
