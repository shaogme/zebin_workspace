use core::{num::NonZeroUsize, str, task::Poll};

use alloc::string::{String, ToString};

use crate::{
    byteops,
    ArchiveBuilder, ArchiveState, ArchivedLayout, ArchivedValidate, ArchivedValidationContext,
    ByteSink, LayoutSink, ZebinError,
    core::rel_ptr::RelPtr,
    num::{u32_to_usize, usize_to_u32},
    traits::Archive,
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
        let len = u32_to_usize(self.len, || ZebinError::ValidationError {
            message: "ArchivedString length exceeds usize range".to_string(),
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

impl ArchivedLayout for ArchivedString {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as ArchivedLayout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl ArchivedValidate for ArchivedString {
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let _guard = context.guard()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };

        let len = u32_to_usize(archived.len, || ZebinError::ValidationError {
            message: "ArchivedString length exceeds usize range".to_string(),
            pos: ptr as usize,
        })?;
        if len > 0 {
            let data_ptr = archived
                .ptr
                .as_ref()
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Null pointer in non-empty ArchivedString".to_string(),
                    pos: ptr as usize,
                })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            context.check_range(data_ptr, len)?;

            let bytes = unsafe { core::slice::from_raw_parts(data_ptr, len) };
            str::from_utf8(bytes).map_err(|_| ZebinError::ValidationError {
                message: "Invalid UTF-8 sequence".to_string(),
                pos: data_ptr as usize,
            })?;
        }

        Ok(())
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

impl<'a> ArchiveState for StringArchiveState<'a> {
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
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
    ) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedString {
            ptr,
            len: usize_to_u32(self.len(), || ZebinError::WriteError)?,
        })
    }
}

impl ArchiveBuilder for String {
    type State<'a>
        = StringArchiveState<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
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
    ) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedString {
            ptr,
            len: usize_to_u32(self.len(), || ZebinError::WriteError)?,
        })
    }
}

impl ArchiveBuilder for str {
    type State<'a>
        = StringArchiveState<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        StringArchiveState::new(self.as_bytes())
    }
}
