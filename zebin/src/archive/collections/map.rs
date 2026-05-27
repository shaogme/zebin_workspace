use alloc::collections::BTreeMap;

use crate::prelude::*;

use super::vec::ArchivedVec;

use super::super::iter::OwnedIterSerializer;

pub type BTreeMapSerializer<'a, K, V> = OwnedIterSerializer<'a, BTreeMap<K, V>, (K, V)>;

impl<K: Archive, V: Archive> Archive for BTreeMap<K, V> {
    type Archived = ArchivedVec<'static, (K::Archived, V::Archived)>;
}

impl<K, V> Serialize for BTreeMap<K, V>
where
    K: Serialize + Archive,
    V: Serialize + Archive,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
    for<'a> K: Serialize<Input<'a> = K> + 'a,
    for<'a> V: Serialize<Input<'a> = V> + 'a,
{
    type Input<'a>
        = BTreeMap<K, V>
    where
        Self: 'a;
    type Serializer<'a>
        = BTreeMapSerializer<'a, K, V>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        BTreeMapSerializer::new()
    }
}

impl<K, V> MeasureBody for BTreeMap<K, V>
where
    K: MeasureBody + Archive,
    V: MeasureBody + Archive,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        // Each map entry is serialized as a `(K, V)` tuple — same shape as a sequence
        // of tuples. We approximate that with a sequence whose archived element
        // alignment is the max of K::ALIGNMENT and V::ALIGNMENT (matching how
        // Tuple2Serializer writes K then V back-to-back).
        let mut pos = 0usize;
        let alignment = core::cmp::max(
            <K::Archived as ArchivedLayout>::ALIGNMENT.get(),
            <V::Archived as ArchivedLayout>::ALIGNMENT.get(),
        );
        let fixed = matches!(
            (
                <K::Archived as ArchivedLayout>::FIXED_SIZE,
                <V::Archived as ArchivedLayout>::FIXED_SIZE,
            ),
            (Some(_), Some(_))
        );
        for (k, v) in self.iter() {
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
                .checked_add(k.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            pos = pos
                .checked_add(v.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        pos = pos
            .checked_add(1)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        Ok(pos)
    }
}

impl<K, V, UK, UV> Deserialize<BTreeMap<UK, UV>> for ArchivedVec<'_, (K, V)>
where
    K: Deserialize<UK>,
    V: Deserialize<UV>,
    UK: Ord,
{
    fn deserialize(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        for item in self.iter() {
            let (k, v) = item.deserialize()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

impl<K, V, UK, UV> Deserialize<BTreeMap<UK, UV>> for BTreeMap<K, V>
where
    K: Deserialize<UK>,
    V: Deserialize<UV>,
    UK: Ord,
{
    fn deserialize(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        for (k, v) in self {
            map.insert(k.deserialize()?, v.deserialize()?);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
pub type HashMapSerializer<'a, K, V> =
    OwnedIterSerializer<'a, std::collections::HashMap<K, V>, (K, V)>;

#[cfg(feature = "std")]
impl<K: Archive, V: Archive> Archive for std::collections::HashMap<K, V> {
    type Archived = ArchivedVec<'static, (K::Archived, V::Archived)>;
}

#[cfg(feature = "std")]
impl<K, V> Serialize for std::collections::HashMap<K, V>
where
    K: Serialize + Archive,
    V: Serialize + Archive,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
    for<'a> K: Serialize<Input<'a> = K> + 'a,
    for<'a> V: Serialize<Input<'a> = V> + 'a,
{
    type Input<'a>
        = std::collections::HashMap<K, V>
    where
        Self: 'a;
    type Serializer<'a>
        = HashMapSerializer<'a, K, V>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        HashMapSerializer::new()
    }
}

#[cfg(feature = "std")]
impl<K, V> MeasureBody for std::collections::HashMap<K, V>
where
    K: MeasureBody + Archive,
    V: MeasureBody + Archive,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let mut pos = 0usize;
        let alignment = core::cmp::max(
            <K::Archived as ArchivedLayout>::ALIGNMENT.get(),
            <V::Archived as ArchivedLayout>::ALIGNMENT.get(),
        );
        let fixed = matches!(
            (
                <K::Archived as ArchivedLayout>::FIXED_SIZE,
                <V::Archived as ArchivedLayout>::FIXED_SIZE,
            ),
            (Some(_), Some(_))
        );
        for (k, v) in self.iter() {
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
                .checked_add(k.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
            pos = pos
                .checked_add(v.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        pos = pos
            .checked_add(1)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        Ok(pos)
    }
}

#[cfg(feature = "std")]
impl<K, V, UK, UV> Deserialize<std::collections::HashMap<UK, UV>> for ArchivedVec<'_, (K, V)>
where
    K: Deserialize<UK>,
    V: Deserialize<UV>,
    UK: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<std::collections::HashMap<UK, UV>, ZebinError> {
        let mut map = std::collections::HashMap::with_capacity(self.len());
        for item in self.iter() {
            let (k, v) = item.deserialize()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
impl<K, V, UK, UV> Deserialize<std::collections::HashMap<UK, UV>>
    for std::collections::HashMap<K, V>
where
    K: Deserialize<UK>,
    V: Deserialize<UV>,
    UK: Eq + core::hash::Hash,
{
    fn deserialize(&self) -> Result<std::collections::HashMap<UK, UV>, ZebinError> {
        let mut map = std::collections::HashMap::with_capacity(self.len());
        for (k, v) in self {
            map.insert(k.deserialize()?, v.deserialize()?);
        }
        Ok(map)
    }
}
