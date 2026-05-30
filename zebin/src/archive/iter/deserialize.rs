#[cfg(any(feature = "alloc", feature = "std"))]
use crate::{error::ZebinError, prelude::*};

#[cfg(any(feature = "alloc", feature = "std"))]
use super::access::OffsetSliceCursor;

#[cfg(feature = "alloc")]
use alloc::{
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    vec::Vec,
};

#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};

#[cfg(any(feature = "alloc", feature = "std"))]
use super::{DummyContext, IterArchive, access::ArchivedIterView};

#[cfg(feature = "alloc")]
pub(crate) fn access_next_element<'a, T: Access, Cr>(
    cursor: &mut Cr,
) -> Result<T::View<'a>, ZebinError>
where
    Cr: Cursor<'a> + ?Sized,
{
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
impl<'a, T, U> Deserialize<Vec<U>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<U>,
{
    fn deserialize(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len);
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            out.push(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, U> Deserialize<VecDeque<U>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<U>,
{
    fn deserialize(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut out = VecDeque::with_capacity(self.len);
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            out.push_back(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, U> Deserialize<BTreeSet<U>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BTreeSet<U>, ZebinError> {
        let mut out = BTreeSet::new();
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            out.insert(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, U> Deserialize<BinaryHeap<U>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<U>,
    U: Ord,
{
    fn deserialize(&self) -> Result<BinaryHeap<U>, ZebinError> {
        let mut out = BinaryHeap::with_capacity(self.len);
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            out.push(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "std")]
impl<'a, T, U> Deserialize<HashSet<U>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<U>,
    U: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<HashSet<U>, ZebinError> {
        let mut out = HashSet::with_capacity(self.len);
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            out.insert(view.deserialize()?);
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, I, U> Deserialize<IterArchive<I, U>> for ArchivedIterView<'a, T>
where
    Self: Deserialize<I>,
{
    fn deserialize(&self) -> Result<IterArchive<I, U>, ZebinError> {
        Ok(IterArchive::new(self.deserialize()?))
    }
}

#[cfg(feature = "alloc")]
impl<'a, T, UK, UV> Deserialize<BTreeMap<UK, UV>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<(UK, UV)>,
    UK: Ord,
{
    fn deserialize(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            let (k, v) = view.deserialize()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
impl<'a, T, UK, UV> Deserialize<HashMap<UK, UV>> for ArchivedIterView<'a, T>
where
    T: Access,
    for<'b> T::View<'b>: Deserialize<(UK, UV)>,
    UK: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<HashMap<UK, UV>, ZebinError> {
        let mut map = HashMap::with_capacity(self.len);
        let mut cursor = OffsetSliceCursor::new(self.source, self.start_pos);
        for _ in 0..self.len {
            let view = access_next_element::<T, _>(&mut cursor)?;
            let (k, v) = view.deserialize()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}
