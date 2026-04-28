use core::num::NonZeroUsize;

use alloc::{boxed::Box, collections::VecDeque, string::ToString, vec::Vec};

use crate::{
    Archive, Encoder, SequenceResolverBuffer, SequenceSerialize, SequenceSource, Serialize,
    SerializePoll, SerializeState, Validate, ZebinError,
    core::{rel_ptr::RelPtr, validator::Validator},
    num::{u32_to_usize, usize_to_u32},
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

    fn poll_children<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<()>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        loop {
            if self.index >= self.len {
                return Ok(SerializePoll::Ready(()));
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
                SerializePoll::Pending => return Ok(SerializePoll::Pending),
                SerializePoll::Error(err) => return Ok(SerializePoll::Error(err)),
                SerializePoll::Ready(resolver) => {
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
pub struct SequenceSerializeState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    children: SequenceChildDriver<'a, S, T, SequenceResolverVec<T>>,
    phase: SequencePhase,
    write_index: usize,
    data_pos: Option<usize>,
    current_bytes: Option<Vec<u8>>,
    current_cursor: usize,
}

impl<'a, S, T> SequenceSerializeState<'a, S, T>
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
            current_bytes: None,
            current_cursor: 0,
        }
    }

    fn poll_serializing<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<()>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        self.children.poll_children(encoder)
    }

    fn poll_aligning<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<()>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        encoder.align(T::ALIGNMENT)?;
        if !encoder.pos().is_multiple_of(T::ALIGNMENT.get()) {
            return Ok(SerializePoll::Pending);
        }
        self.data_pos = Some(encoder.pos());
        Ok(SerializePoll::Ready(()))
    }

    fn poll_writing<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<usize>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        if self.write_index >= self.children.len() {
            return Ok(SerializePoll::Ready(
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
            self.current_bytes = Some(T::archived_bytes(&archived));
            self.current_cursor = 0;
        }

        let archived_bytes = self
            .current_bytes
            .as_ref()
            .expect("archived element initialized above");
        let written = encoder.write(&archived_bytes[self.current_cursor..])?;
        self.current_cursor += written;
        if self.current_cursor < archived_bytes.len() {
            return Ok(SerializePoll::Pending);
        }

        self.current_bytes = None;
        self.write_index += 1;
        Ok(SerializePoll::Pending)
    }
}

impl<'a, S, T> SerializeState for SequenceSerializeState<'a, S, T>
where
    S: ?Sized + SequenceSource<T>,
    T: Serialize + Archive + 'a,
{
    type Resolver = usize;

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        loop {
            match self.phase {
                SequencePhase::Serializing => match self.poll_serializing(encoder)? {
                    SerializePoll::Pending => return Ok(SerializePoll::Pending),
                    SerializePoll::Error(err) => return Ok(SerializePoll::Error(err)),
                    SerializePoll::Ready(()) => {
                        self.phase = SequencePhase::Aligning;
                    }
                },
                SequencePhase::Aligning => match self.poll_aligning(encoder)? {
                    SerializePoll::Pending => return Ok(SerializePoll::Pending),
                    SerializePoll::Error(err) => return Ok(SerializePoll::Error(err)),
                    SerializePoll::Ready(()) => {
                        self.phase = SequencePhase::Writing;
                    }
                },
                SequencePhase::Writing => match self.poll_writing(encoder)? {
                    SerializePoll::Pending => return Ok(SerializePoll::Pending),
                    SerializePoll::Error(err) => return Ok(SerializePoll::Error(err)),
                    SerializePoll::Ready(resolver) => {
                        self.phase = SequencePhase::Done;
                        return Ok(SerializePoll::Ready(resolver));
                    }
                },
                SequencePhase::Done => {
                    return Ok(SerializePoll::Ready(
                        self.data_pos
                            .expect("data_pos set when entering writing phase"),
                    ));
                }
            }
        }
    }
}

pub struct ArraySerializeState<'a, T, const N: usize>
where
    T: Serialize + Archive + 'a,
{
    children: SequenceChildDriver<'a, [T; N], T, SequenceResolverArray<T, N>>,
}

impl<'a, T, const N: usize> ArraySerializeState<'a, T, N>
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

impl<'a, T, const N: usize> SerializeState for ArraySerializeState<'a, T, N>
where
    T: Serialize + Archive + 'a,
{
    type Resolver = [T::Resolver; N];

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        match self.children.poll_children(encoder)? {
            SerializePoll::Pending => Ok(SerializePoll::Pending),
            SerializePoll::Error(err) => Ok(SerializePoll::Error(err)),
            SerializePoll::Ready(()) => Ok(SerializePoll::Ready(self.finish_resolvers())),
        }
    }
}

pub type VecSerializeState<'a, T> = SequenceSerializeState<'a, [T], T>;
pub type VecDequeSerializeState<'a, T> = SequenceSerializeState<'a, VecDeque<T>, T>;
pub type SliceSerializeState<'a, T> = SequenceSerializeState<'a, [T], T>;

pub(crate) fn resolve_sequence_archive<S, T>(
    source: &S,
    pos: usize,
    resolver: usize,
) -> Result<ArchivedVec<T::Archived>, ZebinError>
where
    S: ?Sized + SequenceSource<T>,
    T: Archive,
{
    let ptr = if source.len() == 0 {
        None
    } else {
        Some(RelPtr::new(pos, resolver)?)
    };
    Ok(ArchivedVec {
        ptr,
        len: usize_to_u32(source.len(), || ZebinError::WriteError)?,
    })
}

pub(crate) fn write_sequence_archive_bytes<T>(archived: &ArchivedVec<T>, out: &mut [u8]) {
    out.fill(0);
    if let Some(ptr) = &archived.ptr {
        out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
    }
    <u32 as Archive>::write_archived_bytes(&archived.len, &mut out[8..12]);
}

impl<'v, T> Validate<Validator<'v>> for ArchivedVec<T>
where
    T: Validate<Validator<'v>>,
{
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    unsafe fn validate(ptr: *const Self, context: &mut Validator<'v>) -> Result<(), ZebinError> {
        let _guard = context.enter()?;
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

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        resolve_sequence_archive(self.as_slice(), pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        write_sequence_archive_bytes(archived, out);
    }
}

impl<T> SequenceSerialize for Vec<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = VecSerializeState<'a, T>
    where
        Self: 'a;

    fn begin_sequence(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VecSerializeState::new(self.as_slice()))
    }
}

impl<T> Serialize for Vec<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <Self as SequenceSerialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.begin_sequence()
    }
}

impl<T> Archive for VecDeque<T>
where
    T: Archive,
{
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        resolve_sequence_archive(self, pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        write_sequence_archive_bytes(archived, out);
    }
}

impl<T> SequenceSerialize for VecDeque<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = VecDequeSerializeState<'a, T>
    where
        Self: 'a;

    fn begin_sequence(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VecDequeSerializeState::new(self))
    }
}

impl<T> Serialize for VecDeque<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = <Self as SequenceSerialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.begin_sequence()
    }
}

impl<T> Archive for [T]
where
    T: Archive,
{
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        resolve_sequence_archive(self, pos, resolver)
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        write_sequence_archive_bytes(archived, out);
    }
}

impl<T> SequenceSerialize for [T]
where
    T: Serialize + Archive,
{
    type State<'a>
        = SliceSerializeState<'a, T>
    where
        Self: 'a;

    fn begin_sequence(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(SliceSerializeState::new(self))
    }
}

impl<T> Serialize for [T]
where
    T: Serialize + Archive,
{
    type State<'a>
        = <Self as SequenceSerialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.begin_sequence()
    }
}

impl<T, const N: usize> Archive for [T; N]
where
    T: Archive,
{
    type Archived = [T::Archived; N];
    type Resolver = [T::Resolver; N];
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        let elem_size = core::mem::size_of::<T::Archived>();
        let mut out = core::mem::MaybeUninit::<[T::Archived; N]>::uninit();
        let out_ptr = out.as_mut_ptr() as *mut T::Archived;
        let mut resolver_iter = resolver.into_iter();

        for (index, item) in self.iter().enumerate() {
            let item_resolver = resolver_iter.next().expect("array resolver length matches");
            let item_pos = pos
                .checked_add(index.checked_mul(elem_size).ok_or(ZebinError::WriteError)?)
                .ok_or(ZebinError::WriteError)?;
            let item = item.resolve(item_pos, item_resolver)?;
            unsafe {
                out_ptr.add(index).write(item);
            }
        }

        Ok(unsafe { out.assume_init() })
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        out.fill(0);
        let elem_size = core::mem::size_of::<T::Archived>();
        if elem_size == 0 {
            return;
        }

        for (index, item) in archived.iter().enumerate() {
            let start = index * elem_size;
            let end = start + elem_size;
            T::write_archived_bytes(item, &mut out[start..end]);
        }
    }
}

impl<T, const N: usize> SequenceSerialize for [T; N]
where
    T: Serialize + Archive,
{
    type State<'a>
        = ArraySerializeState<'a, T, N>
    where
        Self: 'a;

    fn begin_sequence(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(ArraySerializeState::new(self))
    }
}

impl<T, const N: usize> Serialize for [T; N]
where
    T: Serialize + Archive,
{
    type State<'a>
        = <Self as SequenceSerialize>::State<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        self.begin_sequence()
    }
}

impl<'v, T, const N: usize> Validate<Validator<'v>> for [T; N]
where
    T: Validate<Validator<'v>>,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    unsafe fn validate(ptr: *const Self, context: &mut Validator<'v>) -> Result<(), ZebinError> {
        let _guard = context.enter()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let data_ptr = ptr as *const T;
        let elem_size = core::mem::size_of::<T>();

        for index in 0..N {
            let element_ptr = if elem_size == 0 {
                data_ptr
            } else {
                unsafe { data_ptr.add(index) }
            };
            unsafe {
                T::validate(element_ptr, context)?;
            }
        }

        Ok(())
    }
}
