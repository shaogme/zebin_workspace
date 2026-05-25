use crate::{prelude::*, utils::padding_for_alignment};
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

    pub fn advance<C>(&mut self, len: usize, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| context.validation_error("Cursor position overflow", self.pos))?;
        context.check_range(self.pos, len)?;
        self.pos = end;
        Ok(())
    }

    pub fn align<C>(
        &mut self,
        alignment: core::num::NonZeroUsize,
        context: &mut C,
    ) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let padding = padding_for_alignment(self.pos, alignment);
        self.advance(padding, context)
    }

    pub fn read_exact<C>(&mut self, len: usize, context: &mut C) -> Result<&'a [u8], DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.pos;
        self.advance(len, context)?;
        Ok(&self.bytes[start..start + len])
    }

    pub fn peek_exact<C>(&self, len: usize, context: &mut C) -> Result<&'a [u8], DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        context.check_range(self.pos, len)?;
        Ok(&self.bytes[self.pos..self.pos + len])
    }

    fn read_fixed<const N: usize, C>(&mut self, context: &mut C) -> Result<&'a [u8; N], DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.pos;
        self.advance(N, context)?;
        Ok(self.bytes[start..self.pos].try_into().unwrap())
    }

    pub fn read_array<const N: usize, C>(&mut self, context: &mut C) -> Result<[u8; N], DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(*self.read_fixed::<N, C>(context)?)
    }

    pub fn read_u8<C>(&mut self, context: &mut C) -> Result<u8, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(self.read_fixed::<1, C>(context)?[0])
    }

    pub fn read_u16<C>(&mut self, context: &mut C) -> Result<u16, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(u16::from_le_bytes(*self.read_fixed::<2, C>(context)?))
    }

    pub fn read_u32<C>(&mut self, context: &mut C) -> Result<u32, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(u32::from_le_bytes(*self.read_fixed::<4, C>(context)?))
    }

    pub fn read_i8<C>(&mut self, context: &mut C) -> Result<i8, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(self.read_u8(context)? as i8)
    }

    pub fn read_i16<C>(&mut self, context: &mut C) -> Result<i16, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(i16::from_le_bytes(*self.read_fixed::<2, C>(context)?))
    }

    pub fn read_i32<C>(&mut self, context: &mut C) -> Result<i32, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(i32::from_le_bytes(*self.read_fixed::<4, C>(context)?))
    }

    pub fn read_u64<C>(&mut self, context: &mut C) -> Result<u64, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(u64::from_le_bytes(*self.read_fixed::<8, C>(context)?))
    }

    pub fn read_i64<C>(&mut self, context: &mut C) -> Result<i64, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(i64::from_le_bytes(*self.read_fixed::<8, C>(context)?))
    }
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

    pub fn new(bytes: &'a [u8], config: ValidationConfig) -> Result<Self, ZebinError> {
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;

        let mut validator = Validator::with_config(bytes, config, None);
        let mut cursor = Cursor::new(bytes, H::SIZE);
        let root = T::Archived::decode(&mut cursor, &mut validator)?;
        if cursor.pos() != bytes.len() {
            let pos = cursor.pos();
            return Err(validator
                .validation_error(
                    "archive validation failed: trailing bytes detected after root object",
                    pos,
                )
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
        Self::new(bytes, ValidationConfig::default())?.restore()
    }

    pub fn validate(
        bytes: &'a [u8],
        config: ValidationConfig,
        stack: Option<&mut ValidationPathStack>,
    ) -> Result<(), ZebinError> {
        let header = H::parse(bytes)?;
        validate_root_object_encoding::<T, H>(&header)?;
        validate_root::<T>(bytes, H::SIZE, config, stack)
    }
}

fn validate_root<'a, T>(
    bytes: &'a [u8],
    root_pos: usize,
    config: ValidationConfig,
    mut stack: Option<&mut ValidationPathStack>,
) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: Decode<'a>,
{
    let mut cursor = Cursor::new(bytes, root_pos);
    let (result, error_path) = {
        let mut validator = Validator::with_config(bytes, config, stack.as_deref_mut());
        let res = T::Archived::validate(&mut cursor, &mut validator).and_then(|()| {
            if cursor.pos() != bytes.len() {
                Err(validator.validation_error("Trailing bytes after root object", cursor.pos()))
            } else {
                Ok(())
            }
        });
        (res, validator.last_error_path().cloned())
    };

    if let (Some(s), Some(ep)) = (stack, error_path) {
        *s = ep;
    }

    result.map_err(Into::into)
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
    let expected = <T::Archived as ArchivedLayout>::OBJECT_ENCODING;
    if actual != expected {
        return Err(DecodeError::UnexpectedObjectEncoding {
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
