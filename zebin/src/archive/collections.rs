use alloc::{
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    vec::Vec,
};
use core::task::Poll;

use crate::{
    io::{ForwardSequenceStrategy, SequenceDecodeStrategy},
    prelude::*,
};

// Replaced SequenceSource with IterEncoder from iter.rs
use super::iter::IterEncoder;

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

/// Decoded archived vector view.
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

impl<'marker, 'a, A> Decode<'a> for ArchivedVec<'marker, A>
where
    A: Decode<'a>,
{
    type View = ArchivedVec<'a, A::View>;
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let items =
            <A::DecodeStrategy as SequenceDecodeStrategy<'a, A>>::decode_sequence(cursor, context)?;
        Ok(ArchivedVec::new(items))
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        <A::DecodeStrategy as SequenceDecodeStrategy<'a, A>>::validate_sequence(cursor, context)
    }
}

impl<T, U> Restore<Vec<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.items.len());
        for item in &self.items {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

impl<T, U> Restore<Vec<U>> for Vec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in self {
            out.push(item.restore()?);
        }
        Ok(out)
    }
}

pub type VecEncoder<'a, T> = IterEncoder<'a, [T], T, Vec<T>>;
pub type VecDequeEncoder<'a, T> = IterEncoder<'a, VecDeque<T>, T, VecDeque<T>>;
pub type BTreeSetEncoder<'a, T> = IterEncoder<'a, BTreeSet<T>, T, BTreeSet<T>>;
pub type BinaryHeapEncoder<'a, T> = IterEncoder<'a, BinaryHeap<T>, T, BinaryHeap<T>>;

#[cfg(feature = "std")]
pub type HashSetEncoder<'a, T> =
    IterEncoder<'a, std::collections::HashSet<T>, T, std::collections::HashSet<T>>;

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Encode for Vec<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = VecEncoder<'a, T>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        VecEncoder::new(self.as_slice())
    }
}

impl<T> Archive for VecDeque<T>
where
    T: Archive,
{
    type Archived = ArchivedVec<'static, T::Archived>;
}

impl<T> Encode for VecDeque<T>
where
    T: Encode + Archive,
    T::Archived: ArchivedLayout,
{
    type Encoder<'a>
        = VecDequeEncoder<'a, T>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        VecDequeEncoder::new(self)
    }
}

impl<T, U> Restore<VecDeque<U>> for ArchivedVec<'_, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self.iter() {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}

impl<T, U> Restore<VecDeque<U>> for VecDeque<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut queue = VecDeque::with_capacity(self.len());
        for item in self {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}

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

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        BTreeSetEncoder::new(self)
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

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        BinaryHeapEncoder::new(self)
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

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        HashSetEncoder::new(self)
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
