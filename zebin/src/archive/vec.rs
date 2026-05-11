use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::task::Poll;

use crate::{
    archive::slice::{ArchivedVec, SequenceSource, resolve_sequence_archive},
    error::ZebinError,
    read::{ResolvedLayout, get_nested_layout},
    traits::{
        Archive, ArchiveHeader, ByteSink, Layout, LayoutSink, Restore, RestoreFromView, Serialize,
        SerializeState,
    },
};

impl<T> SequenceSource<T> for VecDeque<T> {
    fn len(&self) -> usize {
        self.len()
    }

    fn get(&self, index: usize) -> &T {
        &self[index]
    }
}

/// Buffer for sequence element resolvers while a sequence is being serialized.
pub trait SequenceResolverBuffer<T: Archive>: Sized {
    type Resolver;

    fn new(len: usize) -> Self;

    fn store(&mut self, index: usize, resolver: T::Resolver);

    fn take(&mut self, index: usize) -> T::Resolver;

    fn finish(self, data_pos: usize) -> Self::Resolver;
}

pub fn archived_bytes<L: Layout>(archived: &L) -> Vec<u8> {
    let size = archived.size_hint();
    let mut out = vec![0; size];
    L::write_archived_bytes(archived, &mut out);
    out
}

pub(crate) enum SequencePhase {
    Serializing,
    Aligning,
    Writing,
    Done,
}

pub struct SequenceResolverVec<T: Archive> {
    resolvers: Vec<Option<T::Resolver>>,
}

impl<T: Archive> SequenceResolverBuffer<T> for SequenceResolverVec<T> {
    type Resolver = usize;

    fn new(len: usize) -> Self {
        let mut resolvers = Vec::with_capacity(len);
        resolvers.resize_with(len, || None);
        Self { resolvers }
    }

    fn store(&mut self, index: usize, resolver: T::Resolver) {
        self.resolvers[index] = Some(resolver);
    }

    fn take(&mut self, index: usize) -> T::Resolver {
        self.resolvers[index]
            .take()
            .expect("resolver stored when element serialization completed")
    }

    fn finish(self, data_pos: usize) -> Self::Resolver {
        data_pos
    }
}

pub struct SequenceResolverArray<T: Archive, const N: usize> {
    resolvers: [Option<T::Resolver>; N],
}

impl<T: Archive, const N: usize> SequenceResolverBuffer<T> for SequenceResolverArray<T, N> {
    type Resolver = [T::Resolver; N];

    fn new(len: usize) -> Self {
        debug_assert_eq!(len, N);
        Self {
            resolvers: core::array::from_fn(|_| None),
        }
    }

    fn store(&mut self, index: usize, resolver: T::Resolver) {
        self.resolvers[index] = Some(resolver);
    }

    fn take(&mut self, index: usize) -> T::Resolver {
        self.resolvers[index]
            .take()
            .expect("resolver stored when element serialization completed")
    }

    fn finish(self, _data_pos: usize) -> Self::Resolver {
        let mut resolvers = self.resolvers;
        core::array::from_fn(|index| {
            resolvers[index]
                .take()
                .expect("array resolver stored when element serialization completed")
        })
    }
}

struct SequenceChildDriver<'a, S, T, B>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
    B: SequenceResolverBuffer<T>,
{
    source: &'a S,
    len: usize,
    index: usize,
    current_state: Option<Box<<T as Serialize>::State<'a>>>,
    resolvers: B,
}

impl<'a, S, T, B> SequenceChildDriver<'a, S, T, B>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
    B: SequenceResolverBuffer<T>,
{
    fn new(source: &'a S) -> Self {
        let len = source.len();
        Self {
            source,
            len,
            index: 0,
            current_state: None,
            resolvers: B::new(len),
        }
    }

    fn poll_children<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<()>, ZebinError> {
        loop {
            if self.index >= self.len {
                return Ok(Poll::Ready(()));
            }

            if self.current_state.is_none() {
                self.current_state = Some(Box::new(self.source.get(self.index).begin_serialize()?));
            }

            match self
                .current_state
                .as_mut()
                .expect("state initialized above")
                .poll(encoder)?
            {
                Poll::Pending => return Ok(Poll::Pending),
                Poll::Ready(resolver) => {
                    self.resolvers.store(self.index, resolver);
                    self.current_state = None;
                    self.index += 1;
                }
            }
        }
    }

    fn take_resolver(&mut self, index: usize) -> T::Resolver {
        self.resolvers.take(index)
    }

    fn len(&self) -> usize {
        self.len
    }

    fn source(&self) -> &'a S {
        self.source
    }
}

/// Resumable serialization state for indexed sequence containers with an
/// out-of-line data block.
pub struct SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    children: SequenceChildDriver<'a, S, T, SequenceResolverVec<T>>,
    phase: SequencePhase,
    write_index: usize,
    data_pos: Option<usize>,
    current_bytes: [u8; 64],
    current_len: usize,
    current_bytes_vec: Option<Vec<u8>>,
    current_cursor: usize,
}

impl<'a, S, T> SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    pub(crate) fn new(source: &'a S) -> Self {
        Self {
            children: SequenceChildDriver::new(source),
            phase: SequencePhase::Serializing,
            write_index: 0,
            data_pos: None,
            current_bytes: [0u8; 64],
            current_len: 0,
            current_bytes_vec: None,
            current_cursor: 0,
        }
    }

    fn poll_serializing<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<()>, ZebinError> {
        self.children.poll_children(encoder)
    }

    fn poll_aligning<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<()>, ZebinError> {
        encoder.align(<T::Archived as Layout>::ALIGNMENT)?;
        if !encoder
            .pos()
            .is_multiple_of(<T::Archived as Layout>::ALIGNMENT.get())
        {
            return Ok(Poll::Pending);
        }
        self.data_pos = Some(encoder.pos());
        Ok(Poll::Ready(()))
    }

    fn poll_writing<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<usize>, ZebinError> {
        if self.write_index >= self.children.len() {
            return Ok(Poll::Ready(
                self.data_pos
                    .expect("data_pos set when entering writing phase"),
            ));
        }

        if self.current_bytes_vec.is_none() && self.current_len == 0 {
            let resolver = self.children.take_resolver(self.write_index);
            let element_pos = encoder.pos();
            let archived = self
                .children
                .source()
                .get(self.write_index)
                .resolve(element_pos, resolver)?;
            let size = archived.size_hint();
            if size <= 64 {
                <T::Archived as Layout>::write_archived_bytes(
                    &archived,
                    &mut self.current_bytes[..size],
                );
                self.current_len = size;
            } else {
                self.current_bytes_vec = Some(archived_bytes(&archived));
            }
            self.current_cursor = 0;
        }

        let written = if let Some(ref bytes) = self.current_bytes_vec {
            let written = encoder.write(&bytes[self.current_cursor..])?;
            self.current_cursor += written;
            if self.current_cursor < bytes.len() {
                return Ok(Poll::Pending);
            }
            written
        } else {
            let written =
                encoder.write(&self.current_bytes[self.current_cursor..self.current_len])?;
            self.current_cursor += written;
            if self.current_cursor < self.current_len {
                return Ok(Poll::Pending);
            }
            written
        };

        let _ = written;
        self.current_bytes_vec = None;
        self.current_len = 0;
        self.write_index += 1;
        Ok(Poll::Pending)
    }
}

impl<'a, S, T> SerializeState<'a> for SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        loop {
            match self.phase {
                SequencePhase::Serializing => match self.poll_serializing(encoder)? {
                    Poll::Pending => return Ok(Poll::Pending),
                    Poll::Ready(()) => {
                        self.phase = SequencePhase::Aligning;
                    }
                },
                SequencePhase::Aligning => match self.poll_aligning(encoder)? {
                    Poll::Pending => return Ok(Poll::Pending),
                    Poll::Ready(()) => {
                        self.phase = SequencePhase::Writing;
                    }
                },
                SequencePhase::Writing => match self.poll_writing(encoder)? {
                    Poll::Pending => return Ok(Poll::Pending),
                    Poll::Ready(resolver) => {
                        self.phase = SequencePhase::Done;
                        return Ok(Poll::Ready(resolver));
                    }
                },
                SequencePhase::Done => {
                    return Ok(Poll::Ready(
                        self.data_pos
                            .expect("data_pos set when entering writing phase"),
                    ));
                }
            }
        }
    }
}

pub struct ArrayArchiveState<'a, T, const N: usize>
where
    T: Serialize + Archive + 'a,
{
    children: SequenceChildDriver<'a, [T; N], T, SequenceResolverArray<T, N>>,
}

impl<'a, T, const N: usize> ArrayArchiveState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    pub(crate) fn new(items: &'a [T; N]) -> Self {
        Self {
            children: SequenceChildDriver::new(items),
        }
    }

    fn finish_resolvers(&mut self) -> [T::Resolver; N] {
        let len = self.children.len();
        let resolvers = core::mem::replace(
            &mut self.children.resolvers,
            SequenceResolverArray::new(len),
        );
        resolvers.finish(0)
    }
}

impl<'a, T, const N: usize> SerializeState<'a> for ArrayArchiveState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    type Resolver = [T::Resolver; N];

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        match self.children.poll_children(encoder)? {
            Poll::Pending => Ok(Poll::Pending),
            Poll::Ready(()) => Ok(Poll::Ready(self.finish_resolvers())),
        }
    }
}

pub type VecArchiveState<'a, T> = SequenceArchiveState<'a, [T], T>;
pub type VecDequeArchiveState<'a, T> = SequenceArchiveState<'a, VecDeque<T>, T>;
pub type SliceArchiveState<'a, T> = SequenceArchiveState<'a, [T], T>;

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, crate::error::ArchiveError> {
        resolve_sequence_archive(self.as_slice(), archive_pos, resolver)
    }
}

impl<T> Serialize for Vec<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = VecArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VecArchiveState::new(self.as_slice()))
    }
}

impl<T> Archive for VecDeque<T>
where
    T: Archive,
{
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, crate::error::ArchiveError> {
        resolve_sequence_archive(self, archive_pos, resolver)
    }
}

impl<T> Serialize for VecDeque<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = VecDequeArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VecDequeArchiveState::new(self))
    }
}

impl<T> Serialize for [T]
where
    T: Serialize + Archive,
{
    type State<'a>
        = SliceArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(SliceArchiveState::new(self))
    }
}

impl<T, const N: usize> Serialize for [T; N]
where
    T: Serialize + Archive,
{
    type State<'a>
        = ArrayArchiveState<'a, T, N>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ArrayArchiveState::new(self))
    }
}

impl<T, U> Restore<Vec<U>> for ArchivedVec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Vec<U>, ZebinError> {
        let slice = unsafe { self.as_slice() };
        let mut vec = Vec::with_capacity(slice.len());
        for item in slice {
            vec.push(item.restore()?);
        }
        Ok(vec)
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, Vec<U>, H> for ArchivedVec<T>
where
    T: Restore<U> + for<'b> RestoreFromView<'b, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Vec<U>, ZebinError> {
        let slice = unsafe { self.as_slice() };
        let mut vec = Vec::with_capacity(slice.len());
        for item in slice {
            let item_layout = get_nested_layout(layout, item)?;
            vec.push(item.restore_from_view(&item_layout)?);
        }
        Ok(vec)
    }
}

impl<T, U> Restore<VecDeque<U>> for ArchivedVec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let slice = unsafe { self.as_slice() };
        let mut queue = VecDeque::with_capacity(slice.len());
        for item in slice {
            queue.push_back(item.restore()?);
        }
        Ok(queue)
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, VecDeque<U>, H> for ArchivedVec<T>
where
    T: Restore<U> + for<'b> RestoreFromView<'b, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<VecDeque<U>, ZebinError> {
        let slice = unsafe { self.as_slice() };
        let mut queue = VecDeque::with_capacity(slice.len());
        for item in slice {
            let item_layout = get_nested_layout(layout, item)?;
            queue.push_back(item.restore_from_view(&item_layout)?);
        }
        Ok(queue)
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

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, Vec<U>, H> for Vec<&'a T>
where
    T: RestoreFromView<'a, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in self {
            let item_layout = get_nested_layout(layout, *item)?;
            out.push(item.restore_from_view(&item_layout)?);
        }
        Ok(out)
    }
}

impl<T, U> Restore<VecDeque<U>> for VecDeque<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VecDeque<U>, ZebinError> {
        let mut out = VecDeque::with_capacity(self.len());
        for item in self {
            out.push_back(item.restore()?);
        }
        Ok(out)
    }
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, VecDeque<U>, H> for VecDeque<&'a T>
where
    T: RestoreFromView<'a, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<VecDeque<U>, ZebinError> {
        let mut out = VecDeque::with_capacity(self.len());
        for item in self {
            let item_layout = get_nested_layout(layout, *item)?;
            out.push_back(item.restore_from_view(&item_layout)?);
        }
        Ok(out)
    }
}

impl<T, U> Restore<Vec<U>> for [T]
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

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, Vec<U>, H> for &'a [T]
where
    T: RestoreFromView<'a, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<Vec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for item in *self {
            let item_layout = get_nested_layout(layout, item)?;
            out.push(item.restore_from_view(&item_layout)?);
        }
        Ok(out)
    }
}
