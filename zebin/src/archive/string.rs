use core::{marker::PhantomData, ops::Deref, str, task::Poll};

use alloc::string::{String, ToString};

use crate::{
    core::schema::FieldEncoding,
    core::schema::ObjectEncoding,
    error::{DecodeError, ZebinError},
    read::Cursor,
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Encode, EncodeState, Restore,
        SchemaAware,
    },
    validation::context::ValidationContext,
};

impl SchemaAware for ArchivedStringView<'_> {
    fn pos(&self) -> usize {
        0
    }

    fn stable_schema_key(&self) -> u32 {
        0
    }

    fn schema_revision(&self) -> u32 {
        0
    }
}

/// Zero-sized decode marker for archived strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchivedString;

/// Borrowed archived string view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchivedStringView<'a> {
    value: &'a str,
}

impl<'a> ArchivedStringView<'a> {
    pub unsafe fn as_str(&self) -> &'a str {
        self.value
    }
}

impl Deref for ArchivedStringView<'_> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl ArchivedLayout for ArchivedString {
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::LengthPrefixed;
}

impl<'a> Decode<'a> for ArchivedString {
    type View = ArchivedStringView<'a>;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        let len = cursor.read_u32(context)? as usize;
        let bytes = cursor.read_exact(len, context)?;
        let value = str::from_utf8(bytes)
            .map_err(|_| context.validation_error("Invalid UTF-8 sequence", pos))?;
        Ok(ArchivedStringView { value })
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        let len = cursor.read_u32(context)? as usize;
        let bytes = cursor.read_exact(len, context)?;
        str::from_utf8(bytes)
            .map_err(|_| context.validation_error("Invalid UTF-8 sequence", pos))?;
        Ok(())
    }
}

impl ArchivedDefault for ArchivedStringView<'_> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedStringView<'static> = ArchivedStringView { value: "" };
        &DEFAULT
    }
}

impl Restore<String> for ArchivedStringView<'_> {
    fn restore(&self) -> Result<String, ZebinError> {
        Ok(self.value.to_string())
    }
}

/// Resumable serialization state for `String` and `str`.
pub struct StringArchiveState<'a> {
    bytes: &'a [u8],
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    cursor: usize,
    _marker: PhantomData<&'a str>,
}

impl<'a> StringArchiveState<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Result<Self, ZebinError> {
        let len = u32::try_from(bytes.len()).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "String length exceeds u32 range",
        })?;
        Ok(Self {
            bytes,
            len_prefix: len.to_le_bytes(),
            prefix_cursor: 0,
            cursor: 0,
            _marker: PhantomData,
        })
    }
}

impl<'a> EncodeState<'a> for StringArchiveState<'a> {
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.len_prefix.len() {
            let remaining = self.len_prefix.len() - self.prefix_cursor;
            if encoder
                .write(&self.len_prefix[self.prefix_cursor..])?
                .advance_cursor(&mut self.prefix_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

        let remaining = self.bytes.len() - self.cursor;
        Ok(encoder
            .write(&self.bytes[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }
}

impl Archive for String {
    type Archived = ArchivedString;
}

impl Encode for String {
    type State<'a>
        = StringArchiveState<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        StringArchiveState::new(self.as_bytes())
    }
}

impl Archive for str {
    type Archived = ArchivedString;
}

impl Encode for str {
    type State<'a>
        = StringArchiveState<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        StringArchiveState::new(self.as_bytes())
    }
}

impl Restore<String> for String {
    fn restore(&self) -> Result<String, ZebinError> {
        Ok(self.clone())
    }
}

impl Restore<String> for str {
    fn restore(&self) -> Result<String, ZebinError> {
        Ok(self.to_string())
    }
}
