#[cfg(any(feature = "alloc", feature = "std"))]
use crate::{error::ZebinError, prelude::*};

#[cfg(feature = "alloc")]
use alloc::{
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    vec::Vec,
};

#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

#[cfg(any(feature = "alloc", feature = "std"))]
use super::{DummyContext, IterArchive, decode::ArchivedIter};

#[cfg(feature = "alloc")]
pub(crate) fn access_next_element<'a, T: Access>(
    cursor: &mut Cursor<'a>,
) -> Result<T::View<'a>, ZebinError> {
    let mut context = DummyContext;
    let marker = cursor.read_u8(&mut context)?;
    if marker != 1 {
        return Err(ZebinError::Access(AccessError::ValidationError {
            message: "Invalid sequence marker",
            pos: cursor.pos() - 1,
        }));
    }
    if <T as ArchivedLayout>::FIXED_SIZE.is_some() {
        cursor.align(<T as ArchivedLayout>::ALIGNMENT, &mut context)?;
    }
    Ok(T::access(cursor, &mut context)?)
}

#[cfg(feature = "alloc")]
impl<T, U> Deserialize<Vec<U>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<U>,
{
    fn deserialize(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            out.push(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Deserialize<VecDeque<U>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<U>,
{
    fn deserialize(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut out = VecDeque::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            out.push_back(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Deserialize<BTreeSet<U>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut out = BTreeSet::new();
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            out.insert(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, U> Deserialize<BinaryHeap<U>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut out = BinaryHeap::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            out.push(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<T, U> Deserialize<HashSet<U>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<U>,
    U: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<HashSet<U>, ZebinError> {
        let mut out = HashSet::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            out.insert(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<T, I, U> Deserialize<IterArchive<I, U>> for ArchivedIter<'_, T>
where
    Self: Deserialize<I>,
{
    fn deserialize(&self) -> Result<IterArchive<I, U>, ZebinError> {
        Ok(IterArchive::new(self.deserialize()?))
    }
}

#[cfg(feature = "alloc")]
impl<T, UK, UV> Deserialize<BTreeMap<UK, UV>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<(UK, UV)>,
    UK: Ord,
{
    fn deserialize(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            let (k, v) = view.deserialize()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
impl<T, UK, UV> Deserialize<HashMap<UK, UV>> for ArchivedIter<'_, T>
where
    T: Access,
    for<'a> T::View<'a>: Deserialize<(UK, UV)>,
    UK: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<HashMap<UK, UV>, ZebinError> {
        let mut map = HashMap::with_capacity(self.len);
        let mut cursor = Cursor::new(self.bytes, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T>(&mut cursor)?;
            let (k, v) = view.deserialize()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}
