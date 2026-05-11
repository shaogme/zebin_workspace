use core::{num::NonZeroUsize, str, task::Poll};

use alloc::string::String;

use crate::{
    core::rel_ptr::RelPtr,
    error::{AccessError, ArchiveError, ZebinError},
    read::{Cursor, ResolvedLayout},
    traits::{
        Access, Archive, ArchiveHeader, ArchivedDefault, ByteSink, Layout, LayoutSink, Restore,
        RestoreFromView, Serialize, SerializeState,
    },
    utils::{
        byteops,
        num::{u32_to_usize, usize_add_signed, usize_to_u32},
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
        let len = u32_to_usize(self.len, || AccessError::ValidationError {
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

impl<'a> Access<'a> for ArchivedString {
    type View = &'a Self;

    unsafe fn access<H, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        let pos = cursor.pos();
        guard.check_alignment(pos, Self::ALIGNMENT)?;
        guard.check_range(pos, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*(cursor.bytes().as_ptr().add(pos) as *const Self) };

        let len = u32_to_usize(archived.len, || {
            guard.validation_error("ArchivedString length exceeds usize range", pos)
        })?;
        if len > 0 {
            let rel = archived.ptr.as_ref().ok_or_else(|| {
                guard.validation_error("Null pointer in non-empty ArchivedString", pos)
            })?;
            let ptr_pos = pos + crate::memoffset::offset_of!(ArchivedString, ptr);
            let data_pos = usize_add_signed(ptr_pos, rel.offset(), || {
                guard.validation_error("ArchivedString pointer overflow", pos)
            })?;
            guard.check_range(data_pos, len)?;

            let bytes = &cursor.bytes()[data_pos..data_pos + len];
            str::from_utf8(bytes)
                .map_err(|_| guard.validation_error("Invalid UTF-8 sequence", data_pos))?;
        }
        Ok((archived, core::mem::size_of::<Self>()))
    }
}

impl ArchivedDefault for ArchivedString {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedString = ArchivedString { ptr: None, len: 0 };
        &DEFAULT
    }
}

impl Restore<String> for ArchivedString {
    fn restore(&self) -> Result<String, ZebinError> {
        Ok(unsafe { self.as_str() }.to_string())
    }
}

impl<'a, H: ArchiveHeader> RestoreFromView<'a, String, H> for ArchivedString {
    fn restore_from_view(&self, _layout: &ResolvedLayout<'a, H>) -> Result<String, ZebinError> {
        self.restore()
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

impl Restore<String> for String {
    fn restore(&self) -> Result<String, ZebinError> {
        Ok(self.clone())
    }
}

impl<'a, H: ArchiveHeader> RestoreFromView<'a, String, H> for String {
    fn restore_from_view(&self, _layout: &ResolvedLayout<'a, H>) -> Result<String, ZebinError> {
        Ok(self.clone())
    }
}

impl Restore<String> for str {
    fn restore(&self) -> Result<String, ZebinError> {
        Ok(self.to_string())
    }
}

impl<'a, H: ArchiveHeader> RestoreFromView<'a, String, H> for str {
    fn restore_from_view(&self, _layout: &ResolvedLayout<'a, H>) -> Result<String, ZebinError> {
        Ok(self.to_string())
    }
}
