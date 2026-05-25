use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::task::Poll;

use crate::{
    prelude::*,
    validation::{ValidationContext, ValidationPathSegment},
};

#[cfg(feature = "alloc")]
use alloc::boxed::Box;

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
        let len = cursor.read_u32(context)? as usize;
        if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
            cursor.align(<A as ArchivedLayout>::ALIGNMENT, context)?;
        }
        let start_pos = cursor.pos();
        for index in 0..len {
            let mut guard = context.push_index(index);
            A::validate(cursor, &mut *guard)?;
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
        let len = cursor.read_u32(context)? as usize;
        if <A as ArchivedLayout>::FIXED_SIZE.is_some() {
            cursor.align(<A as ArchivedLayout>::ALIGNMENT, context)?;
        }
        for index in 0..len {
            let mut guard = context.push_index(index);
            A::validate(cursor, &mut *guard)?;
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
        Some(A::decode(&mut self.cursor, &mut context))
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<alloc::vec::Vec<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
{
    fn restore(&self) -> Result<alloc::vec::Vec<U>, ZebinError> {
        let mut out = alloc::vec::Vec::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = T::decode(&mut cursor, &mut context)?;
            out.push(view.restore()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Restore<alloc::collections::VecDeque<U>> for ArchivedIter<'_, T>
where
    for<'a> T: Decode<'a>,
    for<'a> <T as Decode<'a>>::View: Restore<U>,
{
    fn restore(&self) -> Result<alloc::collections::VecDeque<U>, ZebinError> {
        let mut out = alloc::collections::VecDeque::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        let mut context = DummyContext;
        for _ in 0..self.len {
            let view = T::decode(&mut cursor, &mut context)?;
            out.push_back(view.restore()?);
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
type CurrentEncoder<'a, T> = Box<(<T as Encode>::Encoder<'a>, bool)>;

#[cfg(not(feature = "alloc"))]
type CurrentEncoder<'a, T> = (<T as Encode>::Encoder<'a>, bool);

pub struct IterEncoder<'a, S: ?Sized, T, I: ?Sized = S>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    T: Encode + Archive + 'a,
{
    iter: <&'a S as IntoIterator>::IntoIter,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    aligned: bool,
    current_encoder: Option<CurrentEncoder<'a, T>>,
    _phantom: PhantomData<&'a I>,
}

impl<'a, S: ?Sized, T, I: ?Sized> IterEncoder<'a, S, T, I>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    for<'b> <&'b S as IntoIterator>::IntoIter: ExactSizeIterator,
    T: Encode + Archive + 'a,
{
    pub fn new(inner: &'a S) -> Result<Self, ZebinError> {
        let iter = inner.into_iter();
        let len = u32::try_from(iter.len()).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "length exceeds u32 range",
        })?;
        Ok(Self {
            iter,
            len_prefix: len.to_le_bytes(),
            prefix_cursor: 0,
            aligned: false,
            current_encoder: None,
            _phantom: PhantomData,
        })
    }
}

impl<'a, S: ?Sized, T, I: ?Sized> Encoder<'a> for IterEncoder<'a, S, T, I>
where
    for<'b> &'b S: IntoIterator<Item = &'b T>,
    for<'b> <&'b S as IntoIterator>::IntoIter: ExactSizeIterator,
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
        if self.prefix_cursor < self.len_prefix.len() {
            let remaining = self.len_prefix.len() - self.prefix_cursor;
            if sink
                .write(&self.len_prefix[self.prefix_cursor..])?
                .advance_cursor(&mut self.prefix_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

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

        loop {
            if self.current_encoder.is_none() {
                if let Some(item) = self.iter.next() {
                    let mut encoder = item.begin_encode()?;
                    match encoder.input(item, sink)? {
                        Poll::Pending => {
                            #[cfg(feature = "alloc")]
                            {
                                self.current_encoder = Some(Box::new((encoder, true)));
                            }
                            #[cfg(not(feature = "alloc"))]
                            {
                                self.current_encoder = Some((encoder, true));
                            }
                            return Ok(Poll::Pending);
                        }
                        Poll::Ready(()) => {
                            #[cfg(feature = "alloc")]
                            {
                                self.current_encoder = Some(Box::new((encoder, false)));
                            }
                            #[cfg(not(feature = "alloc"))]
                            {
                                self.current_encoder = Some((encoder, false));
                            }
                        }
                    }
                } else {
                    break;
                }
            }

            if let Some(state) = &mut self.current_encoder {
                #[cfg(feature = "alloc")]
                let (encoder, started) = &mut **state;
                #[cfg(not(feature = "alloc"))]
                let (encoder, started) = state;

                if *started {
                    match encoder.poll_pending(sink)? {
                        Poll::Pending => return Ok(Poll::Pending),
                        Poll::Ready(()) => {}
                    }
                }

                #[cfg(feature = "alloc")]
                let (encoder, _) = *self.current_encoder.take().unwrap();
                #[cfg(not(feature = "alloc"))]
                let (encoder, _) = self.current_encoder.take().unwrap();

                let _ = encoder.finish(sink)?;
            }
        }

        Ok(Poll::Ready(()))
    }

    fn finish<Sink: ByteSink + ?Sized>(self, _sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<I, T> Encode for IterArchive<I, T>
where
    for<'a> &'a I: IntoIterator<Item = &'a T>,
    for<'a> <&'a I as IntoIterator>::IntoIter: ExactSizeIterator,
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
