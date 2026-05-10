use core::{num::NonZeroUsize, str, task::Poll};

use alloc::string::String;

use crate::{
    core::rel_ptr::RelPtr,
    error::{AccessError, ArchiveError, ValidateError, ZebinError},
    traits::{
        Access, Archive, ArchivedDefault, ByteSink, Layout, LayoutSink, Serialize, SerializeState,
        Validate,
    },
    utils::{
        byteops,
        num::{u32_to_usize, usize_to_u32},
    },
    validation::context::ValidationContext,
};

/// An archived string that uses a relative pointer.
#[repr(C)]
pub struct ArchivedString {
    pub(crate) ptr: Option<RelPtr<u8>>,
    pub(crate) len: u32,
}

impl ArchivedString {
    /// Access the archived string as a &str.
    ///
    /// # Safety
    /// The caller must ensure the underlying data is valid UTF-8 and the pointer is valid.
    pub unsafe fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        let len = u32_to_usize(self.len, || ValidateError::ValidationError {
            message: "ArchivedString length exceeds usize range",
            pos: self as *const _ as usize,
        })
        .expect("validated archived string length should fit in usize");
        let ptr = self
            .ptr
            .as_ref()
            .expect("non-empty archived string must have a pointer");
        let bytes = unsafe { core::slice::from_raw_parts(ptr.as_ptr(), len) };
        unsafe { str::from_utf8_unchecked(bytes) }
    }
}

impl Layout for ArchivedString {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as Layout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl Validate for ArchivedString {
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };

        let len = u32_to_usize(archived.len, || {
            guard.validation_error("ArchivedString length exceeds usize range", ptr as usize)
        })?;
        if len > 0 {
            let data_ptr = archived.ptr.as_ref().ok_or_else(|| {
                guard.validation_error("Null pointer in non-empty ArchivedString", ptr as usize)
            })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            guard.check_range(data_ptr, len)?;

            let bytes = unsafe { core::slice::from_raw_parts(data_ptr, len) };
            str::from_utf8(bytes)
                .map_err(|_| guard.validation_error("Invalid UTF-8 sequence", data_ptr as usize))?;
        }

        Ok(())
    }
}

impl<'a> Access<'a> for ArchivedString {
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

impl ArchivedDefault for ArchivedString {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedString = ArchivedString { ptr: None, len: 0 };
        &DEFAULT
    }
}

/// Resumable serialization state for `String` and `str`.
pub struct StringArchiveState<'a> {
    bytes: &'a [u8],
    cursor: usize,
    start_pos: Option<usize>,
}

impl<'a> StringArchiveState<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Result<Self, ZebinError> {
        Ok(Self {
            bytes,
            cursor: 0,
            start_pos: None,
        })
    }
}

impl<'a> SerializeState<'a> for StringArchiveState<'a> {
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if self.start_pos.is_none() {
            self.start_pos = Some(encoder.pos());
        }

        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < self.bytes.len() {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(self.start_pos.expect("start_pos set above")))
        }
    }
}

impl Archive for String {
    type Archived = ArchivedString;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        let ptr = if self.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedString {
            ptr,
            len: usize_to_u32(self.len(), || ArchiveError::LengthOverflow {
                pos: archive_pos,
            })?,
        })
    }
}

impl Serialize for String {
    type State<'a>
        = StringArchiveState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        StringArchiveState::new(self.as_bytes())
    }
}

impl Archive for str {
    type Archived = ArchivedString;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        let ptr = if self.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedString {
            ptr,
            len: usize_to_u32(self.len(), || ArchiveError::LengthOverflow {
                pos: archive_pos,
            })?,
        })
    }
}

impl Serialize for str {
    type State<'a>
        = StringArchiveState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        StringArchiveState::new(self.as_bytes())
    }
}
