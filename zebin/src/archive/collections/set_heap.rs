use alloc::collections::{BTreeSet, BinaryHeap};

use crate::prelude::*;

use super::super::iter::IterEncoder;
use super::vec::ArchivedVec;

pub type BTreeSetEncoder<'a, T> = IterEncoder<'a, BTreeSet<T>, T, BTreeSet<T>>;
pub type BinaryHeapEncoder<'a, T> = IterEncoder<'a, BinaryHeap<T>, T, BinaryHeap<T>>;

#[cfg(feature = "std")]
pub type HashSetEncoder<'a, T> =
    IterEncoder<'a, std::collections::HashSet<T>, T, std::collections::HashSet<T>>;

impl<T: Archive> Archive for BTreeSet<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Encode for BTreeSet<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = BTreeSetEncoder<'a, T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        BTreeSetEncoder::new()
    }
}

impl<T, U> Restore<BTreeSet<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
    U: Ord,
{
    fn restore(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut set = BTreeSet::new();
        for item in self.iter() {
            set.insert(item.restore()?);
        }
        Ok(set)
    }
}

impl<T, U> Restore<BTreeSet<U>> for BTreeSet<T>
where
    T: Restore<U>,
    U: Ord,
{
    fn restore(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut set = BTreeSet::new();
        for item in self {
            set.insert(item.restore()?);
        }
        Ok(set)
    }
}

impl<T: Archive> Archive for BinaryHeap<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Encode for BinaryHeap<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = BinaryHeapEncoder<'a, T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        BinaryHeapEncoder::new()
    }
}

impl<T, U> Restore<BinaryHeap<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
    U: Ord,
{
    fn restore(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut heap = BinaryHeap::with_capacity(self.len());
        for item in self.iter() {
            heap.push(item.restore()?);
        }
        Ok(heap)
    }
}

impl<T, U> Restore<BinaryHeap<U>> for BinaryHeap<T>
where
    T: Restore<U>,
    U: Ord,
{
    fn restore(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut heap = BinaryHeap::with_capacity(self.len());
        for item in self {
            heap.push(item.restore()?);
        }
        Ok(heap)
    }
}

#[cfg(feature = "std")]
impl<T: Archive> Archive for std::collections::HashSet<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

#[cfg(feature = "std")]
impl<T> Encode for std::collections::HashSet<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = HashSetEncoder<'a, T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        HashSetEncoder::new()
    }
}

#[cfg(feature = "std")]
impl<T, U> Restore<std::collections::HashSet<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
    U: Eq + core::hash::Hash,
{
    fn restore(&self) -> Result<std::collections::HashSet<U>, ZebinError> {
        let mut set = std::collections::HashSet::with_capacity(self.len());
        for item in self.iter() {
            set.insert(item.restore()?);
        }
        Ok(set)
    }
}

#[cfg(feature = "std")]
impl<T, U> Restore<std::collections::HashSet<U>> for std::collections::HashSet<T>
where
    T: Restore<U>,
    U: Eq + core::hash::Hash,
{
    fn restore(&self) -> Result<std::collections::HashSet<U>, ZebinError> {
        let mut set = std::collections::HashSet::with_capacity(self.len());
        for item in self {
            set.insert(item.restore()?);
        }
        Ok(set)
    }
}
