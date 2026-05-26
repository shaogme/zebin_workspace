use core::{marker::PhantomData, ops::Deref, str, task::Poll};

use alloc::string::{String, ToString};
use alloc::vec::Vec;

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
    type DecodeStrategy = ForwardSequenceStrategy;

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

pub trait ToBytesRef<'a> {
    fn to_bytes_ref(self) -> &'a [u8];
}

impl<'a> ToBytesRef<'a> for &'a String {
    fn to_bytes_ref(self) -> &'a [u8] {
        self.as_bytes()
    }
}

impl<'a> ToBytesRef<'a> for &'a str {
    fn to_bytes_ref(self) -> &'a [u8] {
        self.as_bytes()
    }
}

/// Resumable serialization state for an owned `String`.
///
/// On `input(String)`, the string is moved into the encoder via `into_bytes`,
/// so the original allocation is owned by the encoder while it streams.
pub struct StringEncoder {
    bytes: Vec<u8>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    cursor: usize,
}

impl StringEncoder {
    pub(crate) fn new_empty() -> Self {
        Self {
            bytes: Vec::new(),
            len_prefix: [0; 4],
            prefix_cursor: 0,
            cursor: 0,
        }
    }
}

impl Default for StringEncoder {
    fn default() -> Self {
        Self::new_empty()
    }
}

impl Encoder for StringEncoder {
    type Input = String;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let bytes = item.into_bytes();
        let len = bytes.len() as u32;
        self.bytes = bytes;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
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

        let remaining = self.bytes.len() - self.cursor;
        Ok(sink
            .write(&self.bytes[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

/// Resumable serialization state for borrowed string-like inputs (`&str`).
pub struct StrEncoder<'a, T: ?Sized = str> {
    bytes: &'a [u8],
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    cursor: usize,
    _marker: PhantomData<&'a T>,
}

impl<'a, T> StrEncoder<'a, T>
where
    T: ?Sized,
{
    pub(crate) fn new_empty() -> Self {
        Self {
            bytes: &[],
            len_prefix: [0; 4],
            prefix_cursor: 0,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> Encoder for StrEncoder<'a, T>
where
    T: ?Sized,
    &'a T: ToBytesRef<'a>,
{
    type Input = &'a T;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let bytes = item.to_bytes_ref();
        let len = bytes.len() as u32;
        self.bytes = bytes;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
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

        let remaining = self.bytes.len() - self.cursor;
        Ok(sink
            .write(&self.bytes[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl Archive for String {
    type Archived = ArchivedString;
}

impl Encode for String {
    type Input<'a>
        = String
    where
        Self: 'a;
    type Encoder<'a>
        = StringEncoder
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        StringEncoder::new_empty()
    }
}

impl Archive for str {
    type Archived = ArchivedString;
}

impl Encode for str {
    type Input<'a>
        = &'a str
    where
        Self: 'a;
    type Encoder<'a>
        = StrEncoder<'a, str>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        StrEncoder::new_empty()
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
