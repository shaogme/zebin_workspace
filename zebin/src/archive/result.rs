use alloc::string::ToString;
use core::{mem::MaybeUninit, num::NonZeroUsize, task::Poll};

use crate::{
    error::ZebinError,
    io::sink::{ByteSink, LayoutSink},
    traits::{Access, Archive, Layout, Serialize, SerializeState, Validate},
    utils::byteops,
    validation::context::{ValidationContext, ValidationPathSegment},
};

/// Archived representation for `Result<T, E>`.
#[repr(C)]
pub struct ArchivedResult<T, E> {
    tag: u8,
    ok: MaybeUninit<T>,
    err: MaybeUninit<E>,
}

impl<T, E> ArchivedResult<T, E> {
    pub fn is_ok(&self) -> bool {
        self.tag == 0
    }

    pub fn is_err(&self) -> bool {
        self.tag == 1
    }

    /// Returns the archived success payload if this result is `Ok`.
    ///
    /// # Safety
    /// The caller must ensure the archived result is valid.
    pub unsafe fn as_ok(&self) -> Option<&T> {
        match self.tag {
            0 => Some(unsafe { self.ok.assume_init_ref() }),
            1 => None,
            _ => None,
        }
    }

    /// Returns the archived error payload if this result is `Err`.
    ///
    /// # Safety
    /// The caller must ensure the archived result is valid.
    pub unsafe fn as_err(&self) -> Option<&E> {
        match self.tag {
            0 => None,
            1 => Some(unsafe { self.err.assume_init_ref() }),
            _ => None,
        }
    }
}

impl<T, E> Layout for ArchivedResult<T, E>
where
    T: Layout,
    E: Layout,
{
    const ALIGNMENT: NonZeroUsize = if T::ALIGNMENT.get() >= E::ALIGNMENT.get() {
        T::ALIGNMENT
    } else {
        E::ALIGNMENT
    };

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        out[0] = archived.tag;
        if archived.tag == 0 {
            let value_offset = crate::memoffset::offset_of!(ArchivedResult<T, E>, ok);
            let value = unsafe { archived.ok.assume_init_ref() };
            T::write_archived_bytes(
                value,
                &mut out[value_offset..value_offset + core::mem::size_of::<T>()],
            );
        } else if archived.tag == 1 {
            let value_offset = crate::memoffset::offset_of!(ArchivedResult<T, E>, err);
            let value = unsafe { archived.err.assume_init_ref() };
            E::write_archived_bytes(
                value,
                &mut out[value_offset..value_offset + core::mem::size_of::<E>()],
            );
        }
    }
}

impl<T, E> Validate for ArchivedResult<T, E>
where
    T: Layout + Validate,
    E: Layout + Validate,
{
    unsafe fn validate<C: ValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        match archived.tag {
            0 => {
                let mut path_guard =
                    guard.push_path_segment(ValidationPathSegment::Variant("Ok"))?;
                let value_ptr = archived.ok.as_ptr();
                path_guard.check_alignment(value_ptr as *const u8, T::ALIGNMENT)?;
                path_guard.check_range(value_ptr as *const u8, core::mem::size_of::<T>())?;
                unsafe {
                    T::validate(value_ptr, &mut *path_guard)?;
                }
                Ok(())
            }
            1 => {
                let mut path_guard =
                    guard.push_path_segment(ValidationPathSegment::Variant("Err"))?;
                let value_ptr = archived.err.as_ptr();
                path_guard.check_alignment(value_ptr as *const u8, E::ALIGNMENT)?;
                path_guard.check_range(value_ptr as *const u8, core::mem::size_of::<E>())?;
                unsafe {
                    E::validate(value_ptr, &mut *path_guard)?;
                }
                Ok(())
            }
            _ => Err(ZebinError::ValidationError {
                message: "Invalid Result discriminant".to_string(),
                pos: ptr as usize,
            }),
        }
    }
}

impl<'a, T: 'a, E: 'a> Access<'a> for ArchivedResult<T, E>
where
    T: Layout + Validate,
    E: Layout + Validate,
{
    type View = &'a Self;

    unsafe fn access<C: ValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

/// Resumable serialization state for `Result<T, E>`.
pub enum ResultArchiveState<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    Ok(<T as Serialize>::State<'a>),
    Err(<E as Serialize>::State<'a>),
}

impl<'a, T, E> ResultArchiveState<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    pub(crate) fn new(value: Result<&'a T, &'a E>) -> Result<Self, ZebinError> {
        match value {
            Ok(inner) => Ok(Self::Ok(inner.begin_serialize()?)),
            Err(inner) => Ok(Self::Err(inner.begin_serialize()?)),
        }
    }
}

impl<'a, T, E> SerializeState for ResultArchiveState<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    type Resolver = Result<T::Resolver, E::Resolver>;

    fn poll<R: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut R,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        match self {
            ResultArchiveState::Ok(state) => match state.poll(encoder)? {
                Poll::Pending => Ok(Poll::Pending),
                Poll::Ready(resolver) => Ok(Poll::Ready(Ok(resolver))),
            },
            ResultArchiveState::Err(state) => match state.poll(encoder)? {
                Poll::Pending => Ok(Poll::Pending),
                Poll::Ready(resolver) => Ok(Poll::Ready(Err(resolver))),
            },
        }
    }
}

impl<T, E> Archive for Result<T, E>
where
    T: Archive,
    E: Archive,
{
    type Archived = ArchivedResult<T::Archived, E::Archived>;
    type Resolver = Result<T::Resolver, E::Resolver>;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        match (self, resolver) {
            (Ok(value), Ok(resolver)) => {
                let value_offset =
                    crate::memoffset::offset_of!(ArchivedResult<T::Archived, E::Archived>, ok);
                let archived = value.resolve(archive_pos + value_offset, resolver)?;
                Ok(ArchivedResult {
                    tag: 0,
                    ok: MaybeUninit::new(archived),
                    err: MaybeUninit::uninit(),
                })
            }
            (Err(value), Err(resolver)) => {
                let value_offset =
                    crate::memoffset::offset_of!(ArchivedResult<T::Archived, E::Archived>, err);
                let archived = value.resolve(archive_pos + value_offset, resolver)?;
                Ok(ArchivedResult {
                    tag: 1,
                    ok: MaybeUninit::uninit(),
                    err: MaybeUninit::new(archived),
                })
            }
            _ => Err(ZebinError::WriteError),
        }
    }
}

impl<T, E> Serialize for Result<T, E>
where
    T: Serialize + Archive,
    E: Serialize + Archive,
{
    type State<'a>
        = ResultArchiveState<'a, T, E>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        ResultArchiveState::new(self.as_ref())
    }
}
