use alloc::{collections::VecDeque, vec::Vec};

use crate::{
    io::{ForwardSequenceStrategy, SequenceAccessStrategy},
    prelude::*,
};

use super::super::iter::OwnedIterSerializer;

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

/// Accessd archived vector view.
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

impl<A> Access for ArchivedVec<'_, A>
where
    A: Access,
{
    type View<'a>
        = ArchivedVec<'a, A::View<'a>>
    where
        Self: 'a;
    type AccessStrategy = ForwardSequenceStrategy;

    fn access<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a,
    {
        let items =
            <A::AccessStrategy as SequenceAccessStrategy<A>>::access_sequence(cursor, context)?;
        Ok(ArchivedVec::new(items))
    }

    fn validate<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        <A::AccessStrategy as SequenceAccessStrategy<A>>::validate_sequence(cursor, context)
    }
}

impl<T, U> Deserialize<Vec<U>> for ArchivedVec<'_, T>
where
    T: Deserialize<U>,
{
    fn deserialize(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            out.push(item.deserialize()?);
        }
        Ok(out)
    }
}

impl<T, U> Deserialize<Vec<U>> for Vec<T>
where
    T: Deserialize<U>,
{
    fn deserialize(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in self {
            out.push(item.deserialize()?);
        }
        Ok(out)
    }
}

pub type VecSerializer<'a, T> = OwnedIterSerializer<'a, Vec<T>, T>;
pub type VecDequeSerializer<'a, T> = OwnedIterSerializer<'a, VecDeque<T>, T>;

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Serialize for Vec<T>
where
    T: Serialize + Archive,
    T::Archived: ArchivedLayout,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Vec<T>
    where
        Self: 'a;
    type Serializer<'a>
        = VecSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        VecSerializer::new()
    }
}

impl<T> MeasureBody for Vec<T>
where
    T: MeasureBody + Archive,
    T::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        measure_seq_body::<T>(self.iter())
    }
}

pub(crate) fn measure_seq_body<'a, T>(
    items: impl Iterator<Item = &'a T>,
) -> Result<usize, ZebinError>
where
    T: MeasureBody + Archive + 'a,
    T::Archived: ArchivedLayout,
{
    let mut pos = 0usize;
    let alignment = <T::Archived as ArchivedLayout>::ALIGNMENT.get();
    let fixed = <T::Archived as ArchivedLayout>::FIXED_SIZE.is_some();
    for item in items {
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

impl<T: Archive> Archive for VecDeque<T>
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Serialize for VecDeque<T>
where
    T: Serialize + Archive,
    T::Archived: ArchivedLayout,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = VecDeque<T>
    where
        Self: 'a;
    type Serializer<'a>
        = VecDequeSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        VecDequeSerializer::new()
    }
}

impl<T> MeasureBody for VecDeque<T>
where
    T: MeasureBody + Archive,
    T::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        measure_seq_body::<T>(self.iter())
    }
}

impl<T, U> Deserialize<VecDeque<U>> for ArchivedVec<'_, T>
where
    T: Deserialize<U>,
{
    fn deserialize(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self.iter() {
            queue.push_back(item.deserialize()?);
        }
        Ok(queue)
    }
}

impl<T, U> Deserialize<VecDeque<U>> for VecDeque<T>
where
    T: Deserialize<U>,
{
    fn deserialize(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self {
            queue.push_back(item.deserialize()?);
        }
        Ok(queue)
    }
}

impl<'a, T: 'a> ArchivedField<'a> for ArchivedVec<'a, T> {}
