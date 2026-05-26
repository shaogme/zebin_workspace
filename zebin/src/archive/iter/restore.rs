#[cfg(any(feature = "alloc", feature = "std"))]
use crate::{error::ZebinError, prelude::*, read_impl::Cursor};

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
pub(crate) fn decode_next_element<'a, T: Decode<'a>>(
    cursor: &mut Cursor<'a>,
) -> Result<T::View, ZebinError> {
    let mut context = DummyContext;
    let marker = cursor.read_u8(&mut context)?;
    if marker != 1 {
        return Err(ZebinError::Decode(DecodeError::ValidationError {
            message: "Invalid sequence marker",
            pos: cursor.pos() - 1,
        }));
    }
    if <T as ArchivedLayout>::FIXED_SIZE.is_some() {
        cursor.align(<T as ArchivedLayout>::ALIGNMENT, &mut context)?;
    }
    Ok(T::decode(cursor, &mut context)?)
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
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
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
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
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
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
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
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
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
        for _ in 0..self.len {
            let view = decode_next_element::<T>(&mut cursor)?;
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
        for _ in 0..self.len {
            let (k, v) = decode_next_element::<T>(&mut cursor)?;
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
        for _ in 0..self.len {
            let (k, v) = decode_next_element::<T>(&mut cursor)?;
            map.insert(k.restore()?, v.restore()?);
        }
        Ok(map)
    }
}
