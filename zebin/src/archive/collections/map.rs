use alloc::collections::BTreeMap;

use crate::prelude::*;
use core::task::Poll;

use super::vec::ArchivedVec;

use super::super::iter::SeqEncoder;

pub struct MapEntryRef<'a, K, V>(pub &'a K, pub &'a V);

impl<'a, K: Archive, V: Archive> Archive for MapEntryRef<'a, K, V> {
    type Archived = (K::Archived, V::Archived);
}

pub struct MapEntryEncoder<'b, 'a, K: Encode + Archive + 'b, V: Encode + Archive + 'b> {
    item: Option<&'b MapEntryRef<'a, K, V>>,
    key_encoder: Option<(<K as Encode>::Encoder<'b>, bool)>,
    value_encoder: Option<(<V as Encode>::Encoder<'b>, bool)>,
    stage: u8,
}

impl<'b, 'a, K, V> Encoder<'b> for MapEntryEncoder<'b, 'a, K, V>
where
    K: Encode + Archive + 'b,
    V: Encode + Archive + 'b,
{
    type Input = &'b MapEntryRef<'a, K, V>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.item = Some(item);
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        let item = self.item.ok_or(ZebinError::SerializationError {
            pos: sink.pos(),
            message: "MapEntryEncoder polled before input",
        })?;
        if self.stage == 0
            && let Some((encoder, started)) = &mut self.key_encoder
        {
            let progress = if !*started {
                match encoder.input(item.0, sink)? {
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
                    let (enc, _) = self.key_encoder.take().unwrap();
                    let _ = enc.finish(sink)?;
                    self.stage = 1;
                }
            }
        }

        if self.stage == 1
            && let Some((encoder, started)) = &mut self.value_encoder
        {
            let progress = if !*started {
                match encoder.input(item.1, sink)? {
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
                    let (enc, _) = self.value_encoder.take().unwrap();
                    let _ = enc.finish(sink)?;
                    self.stage = 2;
                }
            }
        }

        Ok(Poll::Ready(()))
    }

    fn finish<S: ByteSink + ?Sized>(
        self,
        _sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
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

    fn encoder<'b>() -> Self::Encoder<'b>
    where
        Self: 'b,
    {
        MapEntryEncoder {
            item: None,
            key_encoder: Some((K::encoder(), false)),
            value_encoder: Some((V::encoder(), false)),
            stage: 0,
        }
    }
}

pub trait ToMapIter<'a, K: 'a, V: 'a, Iter> {
    fn to_map_iter(self) -> Iter;
}

impl<'a, K: 'a, V: 'a> ToMapIter<'a, K, V, alloc::collections::btree_map::Iter<'a, K, V>>
    for &'a BTreeMap<K, V>
{
    fn to_map_iter(self) -> alloc::collections::btree_map::Iter<'a, K, V> {
        self.iter()
    }
}

#[cfg(feature = "std")]
impl<'a, K: 'a, V: 'a> ToMapIter<'a, K, V, std::collections::hash_map::Iter<'a, K, V>>
    for &'a std::collections::HashMap<K, V>
{
    fn to_map_iter(self) -> std::collections::hash_map::Iter<'a, K, V> {
        self.iter()
    }
}

pub struct MapEncoder<'a, K, V, Iter, I: ?Sized = ()>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
    Iter: Iterator<Item = (&'a K, &'a V)>,
{
    iter: Option<Iter>,
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
    pub fn new() -> Self {
        Self {
            iter: None,
            seq_encoder: SeqEncoder::new(),
            current_entry: None,
            finished: false,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, K, V, Iter, I: ?Sized> Encoder<'a> for MapEncoder<'a, K, V, Iter, I>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
    K::Archived: ArchivedLayout,
    V::Archived: ArchivedLayout,
    Iter: Iterator<Item = (&'a K, &'a V)>,
    &'a I: ToMapIter<'a, K, V, Iter>,
{
    type Input = &'a I;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        self.iter = Some(item.to_map_iter());
        self.poll_pending(sink)
    }

    fn poll_pending<Sink: ByteSink + ?Sized>(
        &mut self,
        sink: &mut Sink,
    ) -> Result<core::task::Poll<()>, ZebinError> {
        let iter = self.iter.as_mut().ok_or(ZebinError::SerializationError {
            pos: sink.pos(),
            message: "MapEncoder polled before input",
        })?;
        loop {
            if self.seq_encoder.poll_pending(sink)?.is_pending() {
                return Ok(core::task::Poll::Pending);
            }

            if self.seq_encoder.is_finished() {
                return Ok(core::task::Poll::Ready(()));
            }

            if !self.finished {
                if let Some((k, v)) = iter.next() {
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

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        BTreeMapEncoder::new()
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

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        HashMapEncoder::new()
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
