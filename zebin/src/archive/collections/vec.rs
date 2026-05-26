use alloc::{collections::VecDeque, vec::Vec};

use crate::{
    io::{ForwardSequenceStrategy, SequenceDecodeStrategy},
    prelude::*,
};

use super::super::iter::IterEncoder;

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
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let items =
            <A::DecodeStrategy as SequenceDecodeStrategy<'a, A>>::decode_sequence(cursor, context)?;
        Ok(ArchivedVec::new(items))
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        <A::DecodeStrategy as SequenceDecodeStrategy<'a, A>>::validate_sequence(cursor, context)
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

pub type VecEncoder<'a, T> = IterEncoder<'a, [T], T, Vec<T>>;
pub type VecDequeEncoder<'a, T> = IterEncoder<'a, VecDeque<T>, T, VecDeque<T>>;

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

impl<T: Archive> Archive for VecDeque<T>
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
