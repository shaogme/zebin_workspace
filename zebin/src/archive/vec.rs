use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::task::Poll;

use crate::traits::Encoder;
use crate::{Cursor, ObjectEncoding, ValidationContext};
use crate::{
    error::{DecodeError, ZebinError},
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Encode, Restore, SchemaAware,
    },
};


/// Source of indexed sequence items.
pub trait SequenceSource<T> {
    fn len(&self) -> usize;

    #[cfg(feature = "alloc")]
    fn get(&self, index: usize) -> &T;
}

impl<T> SequenceSource<T> for [T] {
    fn len(&self) -> usize {
        <[T]>::len(self)
    }

    #[cfg(feature = "alloc")]
    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<T, const N: usize> SequenceSource<T> for [T; N] {
    fn len(&self) -> usize {
        N
    }

    #[cfg(feature = "alloc")]
    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

impl<'a, T> SchemaAware for ArchivedVec<'a, T> {
    fn pos(&self) -> usize {
        0
    }
    fn stable_schema_key(&self) -> u32 {
        0
    }

    fn schema_revision(&self) -> u32 {
        0
    }
}

impl<T> SequenceSource<T> for VecDeque<T> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

/// Decoded archived vector view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedVec<'a, T> {
    items: Vec<T>,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a, T> ArchivedVec<'a, T> {
    pub fn new(items: Vec<T>) -> Self {
        Self {
            items,
            _marker: core::marker::PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub unsafe fn as_slice(&self) -> &[T] {
        self.items.as_slice()
    }

    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.items.iter()
    }
}

impl<'a, T: 'static> ArchivedDefault for ArchivedVec<'a, T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedVec<'static, ()> = ArchivedVec {
            items: Vec::new(),
            _marker: core::marker::PhantomData,
        };
        unsafe { &*(&DEFAULT as *const ArchivedVec<'static, ()> as *const Self) }
    }
}

impl<A> ArchivedLayout for ArchivedVec<'_, A>
where
    A: ArchivedLayout,
{
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
}

impl<'marker, 'a, A> Decode<'a> for ArchivedVec<'marker, A>
where
    A: Decode<'a>,
{
    type View = ArchivedVec<'a, A::View>;
    type DecodeStrategy = crate::traits::ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let items = <A::DecodeStrategy as crate::traits::SequenceDecodeStrategy<'a, A>>::decode_sequence(cursor, len, context)?;
        Ok(ArchivedVec::new(items))
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        <A::DecodeStrategy as crate::traits::SequenceDecodeStrategy<'a, A>>::validate_sequence(cursor, len, context)
    }
}

impl<T, U> Restore<Vec<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

impl<T, U> Restore<Vec<U>> for Vec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in self {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
}

pub struct SequenceEncoder<'a, S, T, I: ?Sized = S>
where
    S: ?Sized + SequenceSource<T>,
    T: Encode + Archive + 'a,
{
    source: &'a S,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    aligned: bool,
    index: usize,
    current_encoder: Option<Box<(<T as Encode>::Encoder<'a>, bool)>>,
    _phantom: core::marker::PhantomData<&'a I>,
}

impl<'a, S, T, I: ?Sized> SequenceEncoder<'a, S, T, I>
where
    S: ?Sized + SequenceSource<T>,
    T: Encode + Archive + 'a,
{
    pub(crate) fn new(source: &'a S) -> Result<Self, ZebinError> {
        let len = u32::try_from(source.len()).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "sequence length exceeds u32 range",
        })?;
        Ok(Self {
            source,
            len_prefix: len.to_le_bytes(),
            prefix_cursor: 0,
            aligned: false,
            index: 0,
            current_encoder: None,
            _phantom: core::marker::PhantomData,
        })
    }

    fn fixed_width() -> bool
    where
        T::Archived: ArchivedLayout,
    {
        <T::Archived as ArchivedLayout>::FIXED_SIZE.is_some()
    }

    fn ensure_current_encoder(&mut self) -> Result<(), ZebinError>
    where
        T::Archived: ArchivedLayout,
    {
        if self.current_encoder.is_some() {
            return Ok(());
        }

        self.current_encoder = Some(Box::new((
            self.source.get(self.index).begin_encode()?,
            false,
        )));
        Ok(())
    }
}

impl<'a, S, T, I> Encoder<'a> for SequenceEncoder<'a, S, T, I>
where
    S: ?Sized + SequenceSource<T>,
    T: Encode + Archive + 'a,
    T::Archived: ArchivedLayout,
    I: ?Sized,
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

        if Self::fixed_width() && !self.aligned {
            if sink
                .align(<T::Archived as ArchivedLayout>::ALIGNMENT)?
                .is_complete()
            {
                self.aligned = true;
            } else {
                return Ok(Poll::Pending);
            }
        }

        while self.index < self.source.len() {
            self.ensure_current_encoder()?;

            let (encoder, started) = &mut **self
                .current_encoder
                .as_mut()
                .expect("current encoder initialized above");

            let progress = if !*started {
                let item = self.source.get(self.index);
                match encoder.input(item, sink)? {
                    Poll::Pending => {
                        *started = true;
                        Poll::Pending
                    }
                    Poll::Ready(()) => Poll::Ready(()),
                }
            } else {
                encoder.poll_pending(sink)?
            };

            match progress {
                Poll::Pending => return Ok(Poll::Pending),
                Poll::Ready(()) => {
                    let (encoder, _) = *self.current_encoder.take().expect("present");
                    let _ = encoder.finish(sink)?;
                    self.index += 1;
                }
            }
        }

        Ok(Poll::Ready(()))
    }

    fn finish<Sink: ByteSink + ?Sized>(self, _sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

pub struct ArrayEncoder<'a, T, const N: usize>
where
    T: Encode + Archive + 'a,
{
    items: &'a [T; N],
    index: usize,
    current_encoder: Option<(<T as Encode>::Encoder<'a>, bool)>,
}

impl<'a, T, const N: usize> ArrayEncoder<'a, T, N>
where
    T: Encode + Archive + 'a,
{
    pub(crate) fn new(items: &'a [T; N]) -> Self {
        Self {
            items,
            index: 0,
            current_encoder: None,
        }
    }
}

impl<'a, T, const N: usize> Encoder<'a> for ArrayEncoder<'a, T, N>
where
    T: Encode + Archive + 'a,
{
    type Input = &'a [T; N];

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
        while self.index < N {
            if self.current_encoder.is_none() {
                self.current_encoder = Some((self.items[self.index].begin_encode()?, false));
            }
            let (encoder, started) = self
                .current_encoder
                .as_mut()
                .expect("array item encoder initialized above");

            let progress = if !*started {
                match encoder.input(&self.items[self.index], sink)? {
                    Poll::Pending => {
                        *started = true;
                        Poll::Pending
                    }
                    Poll::Ready(()) => Poll::Ready(()),
                }
            } else {
                encoder.poll_pending(sink)?
            };

            match progress {
                Poll::Pending => return Ok(Poll::Pending),
                Poll::Ready(()) => {
                    let (encoder, _) = self.current_encoder.take().expect("present");
                    let _ = encoder.finish(sink)?;
                    self.index += 1;
                }
            }
        }
        Ok(Poll::Ready(()))
    }

    fn finish<Sink: ByteSink + ?Sized>(self, _sink: &mut Sink) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

pub type VecEncoder<'a, T> = SequenceEncoder<'a, [T], T, Vec<T>>;
pub type VecDequeEncoder<'a, T> = SequenceEncoder<'a, VecDeque<T>, T, VecDeque<T>>;
pub type SliceEncoder<'a, T> = SequenceEncoder<'a, [T], T, [T]>;

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Encode for Vec<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = VecEncoder<'a, T>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        VecEncoder::new(self.as_slice())
    }
}

impl<T> Archive for VecDeque<T>
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Encode for VecDeque<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = VecDequeEncoder<'a, T>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        VecDequeEncoder::new(self)
    }
}

impl<T> Encode for [T]
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = SliceEncoder<'a, T>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        SliceEncoder::new(self)
    }
}

impl<T, const N: usize> Encode for [T; N]
where
    T: Encode + Archive,
{
    type Encoder<'a>
        = ArrayEncoder<'a, T, N>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(ArrayEncoder::new(self))
    }
}

impl<T, U> Restore<VecDeque<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self.iter() {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}

impl<T, U> Restore<VecDeque<U>> for VecDeque<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}
