use alloc::{boxed::Box, string::ToString};
use core::{mem::MaybeUninit, num::NonZeroUsize, task::Poll};

use crate::{
    ArchiveBuilder, ArchiveState, ArchivedLayout, ArchivedValidate, ArchivedValidationContext,
    ByteSink, LayoutSink, ZebinError, byteops, traits::Archive,
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

impl<T, E> ArchivedLayout for ArchivedResult<T, E>
where
    T: ArchivedLayout,
    E: ArchivedLayout,
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

impl<T, E> ArchivedValidate for ArchivedResult<T, E>
where
    T: ArchivedLayout + ArchivedValidate,
    E: ArchivedLayout + ArchivedValidate,
{
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let _guard = context.guard()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        match archived.tag {
            0 => {
                let value_ptr = archived.ok.as_ptr();
                context.check_alignment(value_ptr as *const u8, T::ALIGNMENT)?;
                context.check_range(value_ptr as *const u8, core::mem::size_of::<T>())?;
                unsafe {
                    T::validate(value_ptr, context)?;
                }
                Ok(())
            }
            1 => {
                let value_ptr = archived.err.as_ptr();
                context.check_alignment(value_ptr as *const u8, E::ALIGNMENT)?;
                context.check_range(value_ptr as *const u8, core::mem::size_of::<E>())?;
                unsafe {
                    E::validate(value_ptr, context)?;
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

/// Resumable serialization state for `Result<T, E>`.
pub enum ResultArchiveState<'a, T, E>
where
    T: ArchiveBuilder + Archive + 'a,
    E: ArchiveBuilder + Archive + 'a,
{
    Ok(Box<<T as ArchiveBuilder>::State<'a>>),
    Err(Box<<E as ArchiveBuilder>::State<'a>>),
}

impl<'a, T, E> ResultArchiveState<'a, T, E>
where
    T: ArchiveBuilder + Archive + 'a,
    E: ArchiveBuilder + Archive + 'a,
{
    pub(crate) fn new(value: Result<&'a T, &'a E>) -> Result<Self, ZebinError> {
        match value {
            Ok(inner) => Ok(Self::Ok(Box::new(inner.begin()?))),
            Err(inner) => Ok(Self::Err(Box::new(inner.begin()?))),
        }
    }
}

impl<'a, T, E> ArchiveState for ResultArchiveState<'a, T, E>
where
    T: ArchiveBuilder + Archive + 'a,
    E: ArchiveBuilder + Archive + 'a,
{
    type Resolver = Result<T::Resolver, E::Resolver>;

    fn poll<R: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut R,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        match self {
            ResultArchiveState::Ok(state) => match state.as_mut().poll(encoder)? {
                Poll::Pending => Ok(Poll::Pending),
                Poll::Ready(resolver) => Ok(Poll::Ready(Ok(resolver))),
            },
            ResultArchiveState::Err(state) => match state.as_mut().poll(encoder)? {
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

impl<T, E> ArchiveBuilder for Result<T, E>
where
    T: ArchiveBuilder + Archive,
    E: ArchiveBuilder + Archive,
{
    type State<'a>
        = ResultArchiveState<'a, T, E>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        ResultArchiveState::new(self.as_ref())
    }
}
