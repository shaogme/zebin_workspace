use alloc::collections::BTreeMap;

use crate::prelude::*;

use super::vec::ArchivedVec;

use super::super::iter::SeqEncoder;
use super::super::primitive::Tuple2Encoder;

pub struct MapEntryRef<'a, K, V>(&'a K, &'a V);

impl<'a, K: Archive, V: Archive> Archive for MapEntryRef<'a, K, V> {
    type Archived = (K::Archived, V::Archived);
}

pub struct MapEntryEncoder<'b, 'a, K: Encode + Archive + 'b, V: Encode + Archive + 'b> {
    inner: Tuple2Encoder<'b, K, V>,
    _phantom: core::marker::PhantomData<&'b MapEntryRef<'a, K, V>>,
}

impl<'b, 'a, K, V> Encoder<'b> for MapEntryEncoder<'b, 'a, K, V>
where
    K: Encode + Archive + 'b,
    V: Encode + Archive + 'b,
{
    type Input = &'b MapEntryRef<'a, K, V>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish<S: ByteSink + ?Sized>(
        self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<'a, K, V> Encode for MapEntryRef<'a, K, V>
where
    K: Encode + Archive,
    V: Encode + Archive,
{
    type Encoder<'b>
        = MapEntryEncoder<'b, 'a, K, V>
    where
        Self: 'b;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(MapEntryEncoder {
            inner: Tuple2Encoder::new(self.0, self.1)?,
            _phantom: core::marker::PhantomData,
        })
    }
}

pub struct MapEncoder<'a, K, V, Iter, I: ?Sized = ()>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
    Iter: Iterator<Item = (&'a K, &'a V)>,
{
    iter: Iter,
    seq_encoder: SeqEncoder<'a, MapEntryRef<'a, K, V>>,
    current_entry: Option<MapEntryRef<'a, K, V>>,
    finished: bool,
    _phantom: core::marker::PhantomData<&'a I>,
}

impl<'a, K, V, Iter, I: ?Sized> MapEncoder<'a, K, V, Iter, I>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
    Iter: Iterator<Item = (&'a K, &'a V)>,
{
    pub fn new(iter: Iter) -> Result<Self, ZebinError> {
        Ok(Self {
            iter,
            seq_encoder: SeqEncoder::new(),
            current_entry: None,
            finished: false,
            _phantom: core::marker::PhantomData,
        })
    }
}

impl<'a, K, V, Iter, I: ?Sized> Encoder<'a> for MapEncoder<'a, K, V, Iter, I>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
    Iter: Iterator<Item = (&'a K, &'a V)>,
    I: 'a,
{
    type Input = &'a I;

    fn input<Sink: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: ByteSink + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        loop {
            if self.seq_encoder.poll_pending(sink)?.is_pending() {
                return Ok(core::task::Poll::Pending);
            }

            if self.seq_encoder.is_finished() {
                return Ok(core::task::Poll::Ready(()));
            }

            if !self.finished {
                if let Some((k, v)) = self.iter.next() {
                    self.current_entry = Some(MapEntryRef(k, v));
                    let entry_ptr =
                        self.current_entry.as_ref().unwrap() as *const MapEntryRef<'a, K, V>;
                    let entry_ref: &'a MapEntryRef<'a, K, V> = unsafe { &*entry_ptr };
                    if self.seq_encoder.input(entry_ref, sink)?.is_pending() {
                        return Ok(core::task::Poll::Pending);
                    }
                } else {
                    self.finished = true;
                    if self.seq_encoder.finish_ref(sink)?.is_pending() {
                        return Ok(core::task::Poll::Pending);
                    }
                }
            }
        }
    }

    fn finish<Sink: ByteSink + ?Sized>(
        self,
        _sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        Ok(core::task::Poll::Ready(()))
    }
}

pub type BTreeMapEncoder<'a, K, V> =
    MapEncoder<'a, K, V, alloc::collections::btree_map::Iter<'a, K, V>, BTreeMap<K, V>>;

impl<K: Archive, V: Archive> Archive for BTreeMap<K, V> {
    type Archived = ArchivedVec<'static, (K::Archived, V::Archived)>;
}

impl<K, V> Encode for BTreeMap<K, V>
where
    K: Encode + Archive,
    V: Encode + Archive,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = BTreeMapEncoder<'a, K, V>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(BTreeMapEncoder::new(self.iter())?)
    }
}

impl<K, V, UK, UV> Restore<BTreeMap<UK, UV>> for ArchivedVec<'_, (K, V)>
where
    K: Restore<UK>,
    V: Restore<UV>,
    UK: Ord,
{
    fn restore(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        for item in self.iter() {
            let (k, v) = item.restore()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

impl<K, V, UK, UV> Restore<BTreeMap<UK, UV>> for BTreeMap<K, V>
where
    K: Restore<UK>,
    V: Restore<UV>,
    UK: Ord,
{
    fn restore(&self) -> Result<BTreeMap<UK, UV>, ZebinError> {
        let mut map = BTreeMap::new();
        for (k, v) in self {
            map.insert(k.restore()?, v.restore()?);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
pub type HashMapEncoder<'a, K, V> = MapEncoder<
    'a,
    K,
    V,
    std::collections::hash_map::Iter<'a, K, V>,
    std::collections::HashMap<K, V>,
>;

#[cfg(feature = "std")]
impl<K: Archive, V: Archive> Archive for std::collections::HashMap<K, V> {
    type Archived = ArchivedVec<'static, (K::Archived, V::Archived)>;
}

#[cfg(feature = "std")]
impl<K, V> Encode for std::collections::HashMap<K, V>
where
    K: Encode + Archive,
    V: Encode + Archive,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = HashMapEncoder<'a, K, V>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(HashMapEncoder::new(self.iter())?)
    }
}

#[cfg(feature = "std")]
impl<K, V, UK, UV> Restore<std::collections::HashMap<UK, UV>> for ArchivedVec<'_, (K, V)>
where
    K: Restore<UK>,
    V: Restore<UV>,
    UK: Eq + core::hash::Hash,
{
    fn restore(&self) -> Result<std::collections::HashMap<UK, UV>, ZebinError> {
        let mut map = std::collections::HashMap::with_capacity(self.len());
        for item in self.iter() {
            let (k, v) = item.restore()?;
            map.insert(k, v);
        }
        Ok(map)
    }
}

#[cfg(feature = "std")]
impl<K, V, UK, UV> Restore<std::collections::HashMap<UK, UV>> for std::collections::HashMap<K, V>
where
    K: Restore<UK>,
    V: Restore<UV>,
    UK: Eq + core::hash::Hash,
{
    fn restore(&self) -> Result<std::collections::HashMap<UK, UV>, ZebinError> {
        let mut map = std::collections::HashMap::with_capacity(self.len());
        for (k, v) in self {
            map.insert(k.restore()?, v.restore()?);
        }
        Ok(map)
    }
}
