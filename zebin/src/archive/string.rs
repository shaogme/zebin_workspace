use core::{ops::Deref, str, task::Poll};

use alloc::{boxed::Box, rc::Rc, string::String, string::ToString, sync::Arc};

#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::prelude::*;

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

/// Zero-sized deserialize marker for archived strings.
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

impl Access for ArchivedString {
    type View<'a>
        = ArchivedStringView<'a>
    where
        Self: 'a;
    type AccessStrategy = ForwardSequenceStrategy;

    fn access<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a,
    {
        let pos = cursor.pos();
        let len = cursor.read_u32(context)? as usize;
        let bytes = cursor.read_exact(len, context)?;
        let value = str::from_utf8(bytes)
            .map_err(|_| context.validation_error("Invalid UTF-8 sequence", pos))?;
        Ok(ArchivedStringView { value })
    }

    fn validate<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), AccessError>
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

impl Deserialize<String> for ArchivedStringView<'_> {
    fn deserialize(&self) -> Result<String, ZebinError> {
        Ok(self.value.to_string())
    }
}

impl<'a> ArchivedField<'a> for ArchivedStringView<'a> {}

#[derive(Debug, Clone)]
pub(crate) enum StringBytes<'a> {
    Empty,
    Borrowed(&'a str),
    OwnedString(String),
    OwnedBoxStr(Box<str>),
    OwnedRcStr(Rc<str>),
    OwnedArcStr(Arc<str>),
}

impl StringBytes<'_> {
    fn len(&self) -> usize {
        self.as_str().len()
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_str().as_bytes()
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Empty => "",
            Self::Borrowed(value) => value,
            Self::OwnedString(value) => value.as_str(),
            Self::OwnedBoxStr(value) => value,
            Self::OwnedRcStr(value) => value,
            Self::OwnedArcStr(value) => value,
        }
    }
}

impl<'a> Default for StringBytes<'a> {
    fn default() -> Self {
        Self::Empty
    }
}

/// Resumable serialization state for borrowed or owned string-like inputs.
pub(crate) struct StringBytesSerializer<'a> {
    bytes: StringBytes<'a>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    cursor: usize,
}

impl<'a> StringBytesSerializer<'a> {
    pub(crate) fn new_empty() -> Self {
        Self {
            bytes: StringBytes::Empty,
            len_prefix: [0; 4],
            prefix_cursor: 0,
            cursor: 0,
        }
    }

    fn input_bytes(
        &mut self,
        bytes: StringBytes<'a>,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        let len = bytes.len() as u32;
        self.bytes = bytes;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.cursor = 0;
        self.poll_pending(sink)
    }
}

impl<'a> Default for StringBytesSerializer<'a> {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl<'a> Serializer for StringBytesSerializer<'a> {
    type Input = StringBytes<'a>;

    fn poll_pending(&mut self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.len_prefix.len() {
            let remaining = self.len_prefix.len() - self.prefix_cursor;
            if sink
                .write(&self.len_prefix[self.prefix_cursor..])?
                .advance_cursor(&mut self.prefix_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

        let bytes = self.bytes.as_bytes();
        let remaining = bytes.len() - self.cursor;
        Ok(sink
            .write(&bytes[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish(self, _sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        self.input_bytes(item, sink)
    }
}

pub struct StringSerializer {
    inner: StringBytesSerializer<'static>,
}

impl StringSerializer {
    pub(crate) fn new_empty() -> Self {
        Self {
            inner: StringBytesSerializer::new_empty(),
        }
    }
}

impl Default for StringSerializer {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl Serializer for StringSerializer {
    type Input = String;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.input(StringBytes::OwnedString(item), sink)
    }

    fn poll_pending(&mut self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish(self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl Archive for String {
    type Archived = ArchivedString;
}

impl Serialize for String {
    type Input<'a>
        = String
    where
        Self: 'a;
    type Serializer<'a>
        = StringSerializer
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        StringSerializer::new_empty()
    }
}

impl Archive for str {
    type Archived = ArchivedString;
}

impl Serialize for str {
    type Input<'a>
        = &'a str
    where
        Self: 'a;
    type Serializer<'a>
        = StrRefSerializer<'a>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        StrRefSerializer::new()
    }
}

pub struct StrRefSerializer<'a> {
    inner: StringBytesSerializer<'a>,
}

impl<'a> StrRefSerializer<'a> {
    pub fn new() -> Self {
        Self {
            inner: StringBytesSerializer::new_empty(),
        }
    }
}

impl<'a> Default for StrRefSerializer<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Serializer for StrRefSerializer<'a> {
    type Input = &'a str;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        self.inner.input(StringBytes::Borrowed(item), sink)
    }

    fn poll_pending(&mut self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish(self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl MeasureBody for String {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        4usize
            .checked_add(self.len())
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })
    }
}

impl MeasureBody for str {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        4usize
            .checked_add(self.len())
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })
    }
}

impl Deserialize<String> for String {
    fn deserialize(&self) -> Result<String, ZebinError> {
        Ok(self.clone())
    }
}

impl Deserialize<String> for str {
    fn deserialize(&self) -> Result<String, ZebinError> {
        Ok(self.to_string())
    }
}
