use alloc::{boxed::Box, string::ToString};
use core::{mem::MaybeUninit, num::NonZeroUsize, task::Poll};

use crate::{
    ArchiveBuilder, ArchiveState, ArchivedLayout, ArchivedValidate, ArchivedValidationContext,
    ByteSink, LayoutSink, ZebinError, byteops, traits::Archive,
};

/// Archived representation for `Option<T>`.
#[repr(C)]
pub struct ArchivedOption<T> {
    tag: u8,
    value: MaybeUninit<T>,
}

impl<T> ArchivedOption<T> {
    pub fn is_some(&self) -> bool {
        self.tag == 1
    }

    pub fn is_none(&self) -> bool {
        self.tag == 0
    }

    /// Returns the archived payload if this option is `Some`.
    ///
    /// # Safety
    /// The caller must ensure the archived option is valid.
    pub unsafe fn as_ref(&self) -> Option<&T> {
        match self.tag {
            0 => None,
            1 => Some(unsafe { self.value.assume_init_ref() }),
            _ => None,
        }
    }
}

impl<T> ArchivedLayout for ArchivedOption<T>
where
    T: ArchivedLayout,
{
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        out[0] = archived.tag;
        if archived.tag == 1 {
            let value_offset = crate::memoffset::offset_of!(ArchivedOption<T>, value);
            let value = unsafe { archived.value.assume_init_ref() };
            T::write_archived_bytes(
                value,
                &mut out[value_offset..value_offset + core::mem::size_of::<T>()],
            );
        }
    }
}

impl<T> ArchivedValidate for ArchivedOption<T>
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
        match archived.tag {
            0 => Ok(()),
            1 => {
                let value_ptr = archived.value.as_ptr();
                context.check_alignment(value_ptr as *const u8, T::ALIGNMENT)?;
                context.check_range(value_ptr as *const u8, core::mem::size_of::<T>())?;
                unsafe {
                    T::validate(value_ptr, context)?;
                }
                Ok(())
            }
            _ => Err(ZebinError::ValidationError {
                message: "Invalid Option discriminant".to_string(),
                pos: ptr as usize,
            }),
        }
    }
}

/// Resumable serialization state for `Option<T>`.
pub struct OptionArchiveState<'a, T>
where
    T: ArchiveBuilder + Archive + 'a,
{
    is_some: bool,
    inner: Option<Box<<T as ArchiveBuilder>::State<'a>>>,
}

impl<'a, T> OptionArchiveState<'a, T>
where
    T: ArchiveBuilder + Archive + 'a,
{
    fn new(value: Option<&'a T>) -> Result<Self, ZebinError> {
        match value {
            Some(inner) => Ok(Self {
                is_some: true,
                inner: Some(Box::new(inner.begin()?)),
            }),
            None => Ok(Self {
                is_some: false,
                inner: None,
            }),
        }
    }
}

impl<'a, T> ArchiveState for OptionArchiveState<'a, T>
where
    T: ArchiveBuilder + Archive + 'a,
{
    type Resolver = Option<T::Resolver>;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if !self.is_some {
            return Ok(Poll::Ready(None));
        }

        match self
            .inner
            .as_mut()
            .expect("inner state initialized for Some option")
            .poll(encoder)?
        {
            Poll::Pending => Ok(Poll::Pending),
            Poll::Ready(resolver) => {
                self.is_some = false;
                self.inner = None;
                Ok(Poll::Ready(Some(resolver)))
            }
        }
    }
}

impl<T> Archive for Option<T>
where
    T: Archive,
{
    type Archived = ArchivedOption<T::Archived>;
    type Resolver = Option<T::Resolver>;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        match (self, resolver) {
            (Some(value), Some(resolver)) => {
                let value_offset = crate::memoffset::offset_of!(ArchivedOption<T::Archived>, value);
                let archived = value.resolve(archive_pos + value_offset, resolver)?;
                Ok(ArchivedOption {
                    tag: 1,
                    value: MaybeUninit::new(archived),
                })
            }
            (None, None) => Ok(ArchivedOption {
                tag: 0,
                value: MaybeUninit::uninit(),
            }),
            _ => Err(ZebinError::WriteError),
        }
    }
}

impl<T> ArchiveBuilder for Option<T>
where
    T: ArchiveBuilder + Archive,
{
    type State<'a>
        = OptionArchiveState<'a, T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        OptionArchiveState::new(self.as_ref())
    }
}
