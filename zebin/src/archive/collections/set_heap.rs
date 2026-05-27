use alloc::collections::{BTreeSet, BinaryHeap};

use crate::prelude::*;

use super::super::iter::OwnedIterSerializer;
use super::vec::{ArchivedVec, measure_seq_body};

pub type BTreeSetSerializer<'a, T> = OwnedIterSerializer<'a, BTreeSet<T>, T>;
pub type BinaryHeapSerializer<'a, T> = OwnedIterSerializer<'a, BinaryHeap<T>, T>;

#[cfg(feature = "std")]
pub type HashSetSerializer<'a, T> = OwnedIterSerializer<'a, std::collections::HashSet<T>, T>;

impl<T: Archive> Archive for BTreeSet<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Serialize for BTreeSet<T>
where
    T: Serialize + Archive,
    T::Archived: ArchivedLayout,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = BTreeSet<T>
    where
        Self: 'a;
    type Serializer<'a>
        = BTreeSetSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        BTreeSetSerializer::new()
    }
}

impl<T> MeasureBody for BTreeSet<T>
where
    T: MeasureBody + Archive,
    T::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        measure_seq_body::<T>(self.iter())
    }
}

impl<T, U> Deserialize<BTreeSet<U>> for ArchivedVec<'_, T>
where
    T: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut set = BTreeSet::new();
        for item in self.iter() {
            set.insert(item.deserialize()?);
        }
        Ok(set)
    }
}

impl<T, U> Deserialize<BTreeSet<U>> for BTreeSet<T>
where
    T: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut set = BTreeSet::new();
        for item in self {
            set.insert(item.deserialize()?);
        }
        Ok(set)
    }
}

impl<T: Archive> Archive for BinaryHeap<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Serialize for BinaryHeap<T>
where
    T: Serialize + Archive,
    T::Archived: ArchivedLayout,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = BinaryHeap<T>
    where
        Self: 'a;
    type Serializer<'a>
        = BinaryHeapSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        BinaryHeapSerializer::new()
    }
}

impl<T> MeasureBody for BinaryHeap<T>
where
    T: MeasureBody + Archive,
    T::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        measure_seq_body::<T>(self.iter())
    }
}

impl<T, U> Deserialize<BinaryHeap<U>> for ArchivedVec<'_, T>
where
    T: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut heap = BinaryHeap::with_capacity(self.len());
        for item in self.iter() {
            heap.push(item.deserialize()?);
        }
        Ok(heap)
    }
}

impl<T, U> Deserialize<BinaryHeap<U>> for BinaryHeap<T>
where
    T: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut heap = BinaryHeap::with_capacity(self.len());
        for item in self {
            heap.push(item.deserialize()?);
        }
        Ok(heap)
    }
}

#[cfg(feature = "std")]
impl<T: Archive> Archive for std::collections::HashSet<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

#[cfg(feature = "std")]
impl<T> Serialize for std::collections::HashSet<T>
where
    T: Serialize + Archive,
    T::Archived: ArchivedLayout,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = std::collections::HashSet<T>
    where
        Self: 'a;
    type Serializer<'a>
        = HashSetSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        HashSetSerializer::new()
    }
}

#[cfg(feature = "std")]
impl<T> MeasureBody for std::collections::HashSet<T>
where
    T: MeasureBody + Archive,
    T::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        measure_seq_body::<T>(self.iter())
    }
}

#[cfg(feature = "std")]
impl<T, U> Deserialize<std::collections::HashSet<U>> for ArchivedVec<'_, T>
where
    T: Deserialize<U>,
    U: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<std::collections::HashSet<U>, ZebinError> {
        let mut set = std::collections::HashSet::with_capacity(self.len());
        for item in self.iter() {
            set.insert(item.deserialize()?);
        }
        Ok(set)
    }
}

#[cfg(feature = "std")]
impl<T, U> Deserialize<std::collections::HashSet<U>> for std::collections::HashSet<T>
where
    T: Deserialize<U>,
    U: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<std::collections::HashSet<U>, ZebinError> {
        let mut set = std::collections::HashSet::with_capacity(self.len());
        for item in self {
            set.insert(item.deserialize()?);
        }
        Ok(set)
    }
}
