use alloc::{boxed::Box, collections::VecDeque, string::ToString, vec::Vec};
use core::{num::NonZeroUsize, task::Poll};

use crate::{
    ArchiveBuilder, ArchiveState, ArchivedLayout, ArchivedValidate, ArchivedValidationContext,
    ByteSink, LayoutSink, ZebinError,
    core::rel_ptr::RelPtr,
    traits::{Archive, ArchivedDecode, SequenceResolverBuffer, SequenceSource, archived_bytes},
    utils::{
        byteops,
        num::{u32_to_usize, usize_to_u32},
    },
};

/// An archived vector that uses a relative pointer.
#[repr(C)]
pub struct ArchivedVec<T> {
    pub(crate) ptr: Option<RelPtr<T>>,
    pub(crate) len: u32,
}

impl<T> ArchivedVec<T> {
    /// Access the archived vector as a slice.
    ///
    /// # Safety
    /// The caller must ensure the pointer and length are valid.
    pub unsafe fn as_slice(&self) -> &[T] {
        if self.len == 0 {
            return &[];
        }
        let len = u32_to_usize(self.len, || ZebinError::ValidationError {
            message: "ArchivedVec length exceeds usize range".to_string(),
            pos: self as *const _ as usize,
        })
        .expect("validated archived vector length should fit in usize");
        let ptr = self
            .ptr
            .as_ref()
            .expect("non-empty archived vector must have a pointer");
        unsafe { core::slice::from_raw_parts(ptr.as_ptr(), len) }
    }

    /// Get the length of the vector.
    pub fn len(&self) -> usize {
        u32_to_usize(self.len, || ZebinError::ValidationError {
            message: "ArchivedVec length exceeds usize range".to_string(),
            pos: self as *const _ as usize,
        })
        .expect("archived vector length should fit in usize")
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<T> ArchivedLayout for ArchivedVec<T>
where
    T: ArchivedLayout,
{
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as ArchivedLayout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl<T> ArchivedValidate for ArchivedVec<T>
where
    T: ArchivedLayout + ArchivedValidate,
{
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let _guard = context.guard()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };

        let len = u32_to_usize(archived.len, || ZebinError::ValidationError {
            message: "ArchivedVec length exceeds usize range".to_string(),
            pos: ptr as usize,
        })?;
        if len > 0 {
            let data_ptr = archived
                .ptr
                .as_ref()
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Null pointer in non-empty ArchivedVec".to_string(),
                    pos: ptr as usize,
                })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            let total_size = len.checked_mul(core::mem::size_of::<T>()).ok_or_else(|| {
                ZebinError::ValidationError {
                    message: "ArchivedVec size overflow".to_string(),
                    pos: ptr as usize,
                }
            })?;
            context.check_range(data_ptr as *const u8, total_size)?;
            context.check_alignment(data_ptr as *const u8, T::ALIGNMENT)?;

            for i in 0..len {
                let element_ptr = unsafe { data_ptr.add(i) };
                unsafe { T::validate(element_ptr, context)? };
            }
        }

        Ok(())
    }
}

impl<'a, T: 'a> ArchivedDecode<'a> for ArchivedVec<T>
where
    T: ArchivedLayout + ArchivedValidate,
{
    type View = &'a Self;

    unsafe fn decode_view<C: ArchivedValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as ArchivedValidate>::validate(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
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
    T: ArchiveBuilder + Archive + 'a,
    B: SequenceResolverBuffer<T>,
{
    source: &'a S,
    len: usize,
    index: usize,
    current_state: Option<Box<<T as ArchiveBuilder>::State<'a>>>,
    resolvers: B,
}

impl<'a, S, T, B> SequenceChildDriver<'a, S, T, B>
where
    S: ?Sized + SequenceSource<T>,
    T: ArchiveBuilder + Archive + 'a,
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

    fn poll_children<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<()>, ZebinError> {
        loop {
            if self.index >= self.len {
                return Ok(Poll::Ready(()));
            }

            if self.current_state.is_none() {
                self.current_state = Some(Box::new(self.source.get(self.index).begin()?));
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
    T: ArchiveBuilder + Archive + 'a,
{
    children: SequenceChildDriver<'a, S, T, SequenceResolverVec<T>>,
    phase: SequencePhase,
    write_index: usize,
    data_pos: Option<usize>,
    current_bytes: Option<Vec<u8>>,
    current_cursor: usize,
}

impl<'a, S, T> SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: ArchiveBuilder + Archive + 'a,
{
    pub(crate) fn new(source: &'a S) -> Self {
        Self {
            children: SequenceChildDriver::new(source),
            phase: SequencePhase::Serializing,
            write_index: 0,
            data_pos: None,
            current_bytes: None,
            current_cursor: 0,
        }
    }

    fn poll_serializing<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<()>, ZebinError> {
        self.children.poll_children(encoder)
    }

    fn poll_aligning<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<()>, ZebinError> {
        encoder.align(<T::Archived as ArchivedLayout>::ALIGNMENT)?;
        if !encoder
            .pos()
            .is_multiple_of(<T::Archived as ArchivedLayout>::ALIGNMENT.get())
        {
            return Ok(Poll::Pending);
        }
        self.data_pos = Some(encoder.pos());
        Ok(Poll::Ready(()))
    }

    fn poll_writing<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<usize>, ZebinError> {
        if self.write_index >= self.children.len() {
            return Ok(Poll::Ready(
                self.data_pos
                    .expect("data_pos set when entering writing phase"),
            ));
        }

        if self.current_bytes.is_none() {
            let resolver = self.children.take_resolver(self.write_index);
            let element_pos = encoder.pos();
            let archived = self
                .children
                .source()
                .get(self.write_index)
                .resolve(element_pos, resolver)?;
            self.current_bytes = Some(archived_bytes(&archived));
            self.current_cursor = 0;
        }

        let archived_bytes = self
            .current_bytes
            .as_ref()
            .expect("archived element initialized above");
        let written = encoder.write(&archived_bytes[self.current_cursor..])?;
        self.current_cursor += written;
        if self.current_cursor < archived_bytes.len() {
            return Ok(Poll::Pending);
        }

        self.current_bytes = None;
        self.write_index += 1;
        Ok(Poll::Pending)
    }
}

impl<'a, S, T> ArchiveState for SequenceArchiveState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: ArchiveBuilder + Archive + 'a,
{
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
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
    T: ArchiveBuilder + Archive + 'a,
{
    children: SequenceChildDriver<'a, [T; N], T, SequenceResolverArray<T, N>>,
}

impl<'a, T, const N: usize> ArrayArchiveState<'a, T, N>
where
    T: ArchiveBuilder + Archive + 'a,
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

impl<'a, T, const N: usize> ArchiveState for ArrayArchiveState<'a, T, N>
where
    T: ArchiveBuilder + Archive + 'a,
{
    type Resolver = [T::Resolver; N];

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
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

pub(crate) fn resolve_sequence_archive<S, T>(
    source: &S,
    archive_pos: usize,
    resolver: usize,
) -> Result<ArchivedVec<T::Archived>, ZebinError>
where
    S: ?Sized + SequenceSource<T>,
    T: Archive,
{
    let ptr = if source.len() == 0 {
        None
    } else {
        Some(RelPtr::new(archive_pos, resolver)?)
    };
    Ok(ArchivedVec {
        ptr,
        len: usize_to_u32(source.len(), || ZebinError::WriteError)?,
    })
}

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        resolve_sequence_archive(self.as_slice(), archive_pos, resolver)
    }
}

impl<T> ArchiveBuilder for Vec<T>
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = VecArchiveState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
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
    ) -> Result<Self::Archived, ZebinError> {
        resolve_sequence_archive(self, archive_pos, resolver)
    }
}

impl<T> ArchiveBuilder for VecDeque<T>
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = VecDequeArchiveState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VecDequeArchiveState::new(self))
    }
}

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        resolve_sequence_archive(self, archive_pos, resolver)
    }
}

impl<T> ArchiveBuilder for [T]
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = SliceArchiveState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(SliceArchiveState::new(self))
    }
}

impl<T, const N: usize> Archive for [T; N]
where
    T: Archive,
{
    type Archived = [T::Archived; N];
    type Resolver = [T::Resolver; N];

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        let elem_size = core::mem::size_of::<T::Archived>();
        let mut out = core::mem::MaybeUninit::<[T::Archived; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut T::Archived;
        let mut resolver_iter = resolver.into_iter();

        for (index, item) in self.iter().enumerate() {
            let item_resolver = resolver_iter.next().expect("array resolver length matches");
            let item_pos = archive_pos
                .checked_add(index.checked_mul(elem_size).ok_or(ZebinError::WriteError)?)
                .ok_or(ZebinError::WriteError)?;
            let item = item.resolve(item_pos, item_resolver)?;
            unsafe {
                out_ptr.add(index).write(item);
            }
        }

        Ok(unsafe { out.assume_init() })
    }
}

impl<T, const N: usize> ArchiveBuilder for [T; N]
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = ArrayArchiveState<'a, T, N>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ArrayArchiveState::new(self))
    }
}
