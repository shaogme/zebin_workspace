use core::num::NonZeroUsize;

use alloc::{boxed::Box, string::ToString, vec::Vec};

use crate::{
    Archive, Encoder, Serialize, SerializePoll, SerializeState, Validate, ZebinError,
    core::{rel_ptr::RelPtr, validator::Validator},
    num::{u32_to_usize, usize_to_u32},
};

/// An archived vector that uses a relative pointer.
#[repr(C)]
pub struct ArchivedVec<T> {
    ptr: Option<RelPtr<T>>,
    len: u32,
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

enum VecPhase {
    Serializing,
    Aligning,
    Writing,
    Done,
}

/// Resumable serialization state for `Vec<T>`.
pub struct VecSerializeState<'a, T>
where
    T: Serialize + Archive,
{
    items: &'a [T],
    phase: VecPhase,
    serialize_index: usize,
    write_index: usize,
    current_state: Option<Box<<T as Serialize>::State<'a>>>,
    resolvers: Vec<Option<T::Resolver>>,
    data_pos: Option<usize>,
    current_bytes: Option<Vec<u8>>,
    current_cursor: usize,
}

impl<'a, T> VecSerializeState<'a, T>
where
    T: Serialize + Archive,
{
    fn new(items: &'a [T]) -> Result<Self, ZebinError> {
        let mut resolvers = Vec::with_capacity(items.len());
        resolvers.resize_with(items.len(), || None);
        Ok(Self {
            items,
            phase: VecPhase::Serializing,
            serialize_index: 0,
            write_index: 0,
            current_state: None,
            resolvers,
            data_pos: None,
            current_bytes: None,
            current_cursor: 0,
        })
    }
}

impl<'a, T> SerializeState for VecSerializeState<'a, T>
where
    T: Serialize + Archive,
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
                VecPhase::Serializing => {
                    if self.serialize_index >= self.items.len() {
                        self.phase = VecPhase::Aligning;
                        self.write_index = 0;
                        continue;
                    }

                    if self.current_state.is_none() {
                        self.current_state =
                            Some(Box::new(self.items[self.serialize_index].begin()?));
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
                            self.resolvers[self.serialize_index] = Some(resolver);
                            self.current_state = None;
                            self.serialize_index += 1;
                        }
                    }
                }
                VecPhase::Aligning => {
                    let _ = encoder.align(T::ALIGNMENT)?;
                    if !encoder.pos().is_multiple_of(T::ALIGNMENT.get()) {
                        return Ok(SerializePoll::Pending);
                    }
                    self.data_pos = Some(encoder.pos());
                    self.phase = VecPhase::Writing;
                }
                VecPhase::Writing => {
                    if self.write_index >= self.items.len() {
                        self.phase = VecPhase::Done;
                        return Ok(SerializePoll::Ready(
                            self.data_pos
                                .expect("data_pos set when entering writing phase"),
                        ));
                    }

                    if self.current_bytes.is_none() {
                        let resolver = self.resolvers[self.write_index]
                            .take()
                            .expect("resolver stored when element serialization completed");
                        let element_pos = encoder.pos();
                        let archived =
                            self.items[self.write_index].resolve(element_pos, resolver)?;
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
                }
                VecPhase::Done => {
                    return Ok(SerializePoll::Ready(
                        self.data_pos
                            .expect("data_pos set when entering writing phase"),
                    ));
                }
            }
        }
    }
}

impl<T: Archive> Archive for Vec<T> {
    type Archived = ArchivedVec<T::Archived>;
    type Resolver = usize;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.is_empty() {
            None
        } else {
            Some(RelPtr::new(pos, resolver)?)
        };
        Ok(ArchivedVec {
            ptr,
            len: usize_to_u32(self.len(), || ZebinError::WriteError)?,
        })
    }

    fn write_archived_bytes(archived: &Self::Archived, out: &mut [u8]) {
        out.fill(0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as Archive>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl<T> Serialize for Vec<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = VecSerializeState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        VecSerializeState::new(self.as_slice())
    }
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
