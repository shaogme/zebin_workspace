use core::num::NonZeroUsize;
use core::str;

use alloc::string::{String, ToString};

use crate::{
    Archive, Encoder, Serialize, SerializePoll, SerializeState, Validate, ZebinError,
    core::{rel_ptr::RelPtr, validator::Validator},
    num::{u32_to_usize, usize_to_u32},
};

/// An archived string that uses a relative pointer.
#[repr(C)]
pub struct ArchivedString {
    ptr: Option<RelPtr<u8>>,
    len: u32,
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

/// Resumable serialization state for `String`.
pub struct StringSerializeState<'a> {
    bytes: &'a [u8],
    cursor: usize,
    start_pos: Option<usize>,
}

impl<'a> StringSerializeState<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, ZebinError> {
        Ok(Self {
            bytes,
            cursor: 0,
            start_pos: None,
        })
    }
}

impl<'a> SerializeState for StringSerializeState<'a> {
    type Resolver = usize;

    fn poll<E: Encoder + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<SerializePoll<Self::Resolver>, E::Error>
    where
        E::Error: From<ZebinError>,
    {
        if self.start_pos.is_none() {
            self.start_pos = Some(encoder.pos());
        }

        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < self.bytes.len() {
            Ok(SerializePoll::Pending)
        } else {
            Ok(SerializePoll::Ready(
                self.start_pos.expect("start_pos set above"),
            ))
        }
    }
}

impl Archive for String {
    type Archived = ArchivedString;
    type Resolver = usize;
    const ALIGNMENT: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(8) };

    fn resolve(&self, pos: usize, resolver: Self::Resolver) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.is_empty() {
            None
        } else {
            Some(RelPtr::new(pos, resolver)?)
        };
        Ok(ArchivedString {
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

impl Serialize for String {
    type State<'a>
        = StringSerializeState<'a>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        StringSerializeState::new(self.as_bytes())
    }
}

impl<'v> Validate<Validator<'v>> for ArchivedString {
    const ALIGNMENT: NonZeroUsize = unsafe { NonZeroUsize::new_unchecked(8) };

    unsafe fn validate(ptr: *const Self, context: &mut Validator<'v>) -> Result<(), ZebinError> {
        let _guard = context.enter()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };

        // Validate the string data pointer and length.
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

            // Validate UTF-8.
            let bytes = unsafe { core::slice::from_raw_parts(data_ptr, len) };
            str::from_utf8(bytes).map_err(|_| ZebinError::ValidationError {
                message: "Invalid UTF-8 sequence".to_string(),
                pos: data_ptr as usize,
            })?;
        }

        Ok(())
    }
}
