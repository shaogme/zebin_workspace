use alloc::collections::BTreeMap;
use core::task::Poll;

use crate::prelude::*;

use super::vec::ArchivedVec;

pub struct TupleRefEncoder<'a, K: Encode + Archive + 'a, V: Encode + Archive + 'a> {
    key: &'a K,
    value: &'a V,
    key_encoder: Option<(<K as Encode>::Encoder<'a>, bool)>,
    value_encoder: Option<(<V as Encode>::Encoder<'a>, bool)>,
    stage: u8,
}

impl<'a, K, V> TupleRefEncoder<'a, K, V>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
{
    pub fn new(key: &'a K, value: &'a V) -> Result<Self, ZebinError> {
        Ok(Self {
            key,
            value,
            key_encoder: Some((key.begin_encode()?, false)),
            value_encoder: Some((value.begin_encode()?, false)),
            stage: 0,
        })
    }

    pub fn poll_pending<S: ByteSink + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if self.stage == 0
            && let Some((encoder, started)) = &mut self.key_encoder
        {
            let progress = if !*started {
                match encoder.input(self.key, sink)? {
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
                match encoder.input(self.value, sink)? {
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
}

pub struct MapEncoder<'a, K, V, Iter, I: ?Sized = ()>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
    Iter: Iterator<Item = (&'a K, &'a V)>,
{
    iter: Iter,
    next_item: Option<(&'a K, &'a V)>,
    marker: [u8; 1],
    marker_cursor: usize,
    current_encoder: Option<TupleRefEncoder<'a, K, V>>,
    finished_sentinel: bool,
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
            next_item: None,
            marker: [0],
            marker_cursor: 1,
            current_encoder: None,
            finished_sentinel: false,
            _phantom: core::marker::PhantomData,
        })
    }
}

impl<'a, K, V, Iter, I: ?Sized> Encoder<'a> for MapEncoder<'a, K, V, Iter, I>
where
    K: Encode + Archive + 'a,
    V: Encode + Archive + 'a,
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
            if self.marker_cursor < 1 {
                let remaining = 1 - self.marker_cursor;
                if sink
                    .write(&self.marker[self.marker_cursor..])?
                    .advance_cursor(&mut self.marker_cursor, remaining)
                    .is_pending()
                {
                    return Ok(core::task::Poll::Pending);
                }
            }

            if self.finished_sentinel && self.marker_cursor == 1 {
                return Ok(core::task::Poll::Ready(()));
            }

            if let Some(encoder) = &mut self.current_encoder {
                match encoder.poll_pending(sink)? {
                    core::task::Poll::Pending => return Ok(core::task::Poll::Pending),
                    core::task::Poll::Ready(()) => {
                        self.current_encoder = None;
                    }
                }
            }

            if self.next_item.is_none() && !self.finished_sentinel {
                if let Some((k, v)) = self.iter.next() {
                    self.next_item = Some((k, v));
                    self.marker = [1];
                    self.marker_cursor = 0;
                } else {
                    self.marker = [0];
                    self.marker_cursor = 0;
                    self.finished_sentinel = true;
                }
                continue;
            }

            if let Some((k, v)) = self.next_item.take() {
                let encoder = TupleRefEncoder::new(k, v)?;
                self.current_encoder = Some(encoder);
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
        BTreeMapEncoder::new(self.iter())
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
        HashMapEncoder::new(self.iter())
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
