use crate::{
    core::schema::ObjectEncoding,
    error::{AccessError, ZebinError},
    format::ArchiveHeader,
    traits::{Archive, ArchiveHeader as ArchiveHeaderTrait, Decode, Restore},
    validation::{context::ValidationContext, validator::Validator},
};
use core::ops::Deref;

/// Borrowed cursor into an archive byte slice.
#[derive(Clone, Copy)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    pub fn with_pos(&self, pos: usize) -> Self {
        Self {
            bytes: self.bytes,
            pos,
        }
    }

    pub fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        context.check_range(self.pos, len)?;
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        if end > self.bytes.len() {
            return Err(context.validation_error("Cursor out of bounds", self.pos));
        }
        self.pos = end;
        Ok(())
    }

    pub fn align<C>(
        &mut self,
        alignment: core::num::NonZeroUsize,
        context: &mut C,
    ) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let padding = padding_for_alignment(self.pos, alignment);
        self.advance(padding, context)
    }

    pub fn read_exact<C>(&mut self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.pos;
        self.advance(len, context)?;
        Ok(&self.bytes[start..start + len])
    }

    pub fn read_u8<C>(&mut self, context: &mut C) -> Result<u8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(self.read_exact(1, context)?[0])
    }

    pub fn read_u16<C>(&mut self, context: &mut C) -> Result<u16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes: [u8; 2] = self.read_exact(2, context)?.try_into().unwrap();
        Ok(u16::from_le_bytes(bytes))
    }

    pub fn read_u32<C>(&mut self, context: &mut C) -> Result<u32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let bytes: [u8; 4] = self.read_exact(4, context)?.try_into().unwrap();
        Ok(u32::from_le_bytes(bytes))
    }
}

pub(crate) fn padding_for_alignment(pos: usize, alignment: core::num::NonZeroUsize) -> usize {
    let alignment = alignment.get();
    (alignment - (pos % alignment)) % alignment
}

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ZebinReader<'a, T: Archive, H: ArchiveHeaderTrait = ArchiveHeader>
where
    T::Archived: Decode<'a>,
{
    bytes: &'a [u8],
    header: H,
    root: <T::Archived as Decode<'a>>::View,
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> ZebinReader<'a, T, H>
where
    T::Archived: Decode<'a>,
{
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> &H {
        &self.header
    }

    pub fn root(&self) -> &<T::Archived as Decode<'a>>::View {
        &self.root
    }

    pub fn restore(&self) -> Result<T, ZebinError>
    where
        <T::Archived as Decode<'a>>::View: Restore<T>,
    {
        self.root.restore()
    }

    pub fn new(bytes: &'a [u8]) -> Result<Self, ZebinError> {
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;
        let mut validator = Validator::new(bytes);
        let mut cursor = Cursor::new(bytes, H::SIZE);
        let root = T::Archived::decode(&mut cursor, &mut validator)?;
        if cursor.pos() != bytes.len() {
            return Err(AccessError::ValidationError {
                message: "Trailing bytes after root object",
                pos: cursor.pos(),
            }
            .into());
        }

        Ok(Self {
            bytes,
            header,
            root,
        })
    }

    pub fn decode(bytes: &'a [u8]) -> Result<T, ZebinError>
    where
        <T::Archived as Decode<'a>>::View: Restore<T>,
    {
        Self::new(bytes)?.restore()
    }

    pub fn validate(bytes: &'a [u8]) -> Result<(), ZebinError> {
        Self::new(bytes).map(|_| ())
    }
}

fn validate_root_object_encoding<'a, T, H>(header: &H) -> Result<(), ZebinError>
where
    T: Archive,
    H: ArchiveHeaderTrait,
    T::Archived: Decode<'a>,
{
    let actual = ObjectEncoding::from_byte(header.flags()).ok_or(
        crate::error::ParseHeaderError::InvalidObjectEncoding {
            flags: header.flags(),
            pos: H::SIZE.saturating_sub(1),
        },
    )?;
    let expected = <T::Archived as Decode<'a>>::OBJECT_ENCODING;
    if actual != expected {
        return Err(AccessError::UnexpectedObjectEncoding {
            expected,
            actual,
            pos: H::SIZE.saturating_sub(1),
        }
        .into());
    }
    Ok(())
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> Deref for ZebinReader<'a, T, H>
where
    T::Archived: Decode<'a>,
{
    type Target = <T::Archived as Decode<'a>>::View;

    fn deref(&self) -> &Self::Target {
        &self.root
    }
}
