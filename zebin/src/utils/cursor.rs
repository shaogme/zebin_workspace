use crate::prelude::*;
use crate::utils::padding_for_alignment;

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

    pub fn peek_exact<C>(&self, len: usize, context: &mut C) -> Result<&'a [u8], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        context.check_range(self.pos, len)?;
        Ok(&self.bytes[self.pos..self.pos + len])
    }

    fn read_fixed<const N: usize, C>(&mut self, context: &mut C) -> Result<&'a [u8; N], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let start = self.pos;
        self.advance(N, context)?;
        Ok(self.bytes[start..self.pos].try_into().unwrap())
    }

    pub fn read_array<const N: usize, C>(&mut self, context: &mut C) -> Result<[u8; N], AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(*self.read_fixed::<N, C>(context)?)
    }

    pub fn read_u8<C>(&mut self, context: &mut C) -> Result<u8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(self.read_fixed::<1, C>(context)?[0])
    }

    pub fn read_u16<C>(&mut self, context: &mut C) -> Result<u16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(u16::from_le_bytes(*self.read_fixed::<2, C>(context)?))
    }

    pub fn read_u32<C>(&mut self, context: &mut C) -> Result<u32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(u32::from_le_bytes(*self.read_fixed::<4, C>(context)?))
    }

    pub fn read_i8<C>(&mut self, context: &mut C) -> Result<i8, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(self.read_u8(context)? as i8)
    }

    pub fn read_i16<C>(&mut self, context: &mut C) -> Result<i16, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(i16::from_le_bytes(*self.read_fixed::<2, C>(context)?))
    }

    pub fn read_i32<C>(&mut self, context: &mut C) -> Result<i32, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(i32::from_le_bytes(*self.read_fixed::<4, C>(context)?))
    }

    pub fn read_u64<C>(&mut self, context: &mut C) -> Result<u64, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(u64::from_le_bytes(*self.read_fixed::<8, C>(context)?))
    }

    pub fn read_i64<C>(&mut self, context: &mut C) -> Result<i64, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(i64::from_le_bytes(*self.read_fixed::<8, C>(context)?))
    }
}
