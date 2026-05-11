use crate::{
    core::schema::FieldEncoding,
    error::{AccessError, ZebinError},
    read::Cursor,
    traits::{Archive, ArchivedDefault, Decode, Restore},
    validation::context::ValidationContext,
};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

pub(crate) fn packed_byte_len(
    value_count: usize,
    bits_per_value: usize,
) -> Result<usize, AccessError> {
    let total_bits =
        value_count
            .checked_mul(bits_per_value)
            .ok_or(AccessError::ValidationError {
                message: "Packed length calculation overflow",
                pos: 0,
            })?;
    Ok(total_bits.div_ceil(8))
}

pub(crate) fn read_packed_bits(bytes: &[u8], bit_offset: usize, bits_per_value: usize) -> u8 {
    let mut value = 0u8;
    for bit in 0..bits_per_value {
        let absolute_bit = bit_offset + bit;
        let byte = bytes[absolute_bit / 8];
        let bit_value = (byte >> (absolute_bit % 8)) & 1;
        value |= bit_value << bit;
    }
    value
}

/// Zero-sized decode marker for archived packed boolean slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivedPackedBoolSlice;

/// Archived packed boolean slice view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivedPackedBoolSliceView<'a> {
    len: usize,
    bytes: &'a [u8],
}

impl<'a> ArchivedPackedBoolSliceView<'a> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<bool> {
        if index >= self.len {
            return None;
        }
        let byte_index = index / 8;
        let bit = index % 8;
        let value = (self.bytes.get(byte_index)? >> bit) & 1;
        Some(value != 0)
    }
}

impl<'a> Decode<'a> for ArchivedPackedBoolSlice {
    type View = ArchivedPackedBoolSliceView<'a>;

    const FIELD_ENCODING: FieldEncoding = FieldEncoding::PackedBits;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let byte_len = packed_byte_len(len, 1)?;
        let bytes = cursor.read_exact(byte_len, context)?;
        Ok(ArchivedPackedBoolSliceView { len, bytes })
    }
}

impl ArchivedDefault for ArchivedPackedBoolSliceView<'_> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedPackedBoolSliceView<'static> =
            ArchivedPackedBoolSliceView { len: 0, bytes: &[] };
        unsafe { &*(&DEFAULT as *const ArchivedPackedBoolSliceView<'static> as *const Self) }
    }
}

/// Zero-sized decode marker for archived packed u8 slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivedPackedU8Slice<const BITS: u8 = 8>;

/// Archived packed small-integer slice view. Values are stored using `BITS` bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivedPackedU8SliceView<'a, const BITS: u8 = 8> {
    len: usize,
    bytes: &'a [u8],
}

impl<'a, const BITS: u8> ArchivedPackedU8SliceView<'a, BITS> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<u8> {
        if index >= self.len {
            return None;
        }
        let bit_offset = index * usize::from(BITS);
        Some(read_packed_bits(self.bytes, bit_offset, usize::from(BITS)))
    }
}

impl<'a, const BITS: u8> Decode<'a> for ArchivedPackedU8Slice<BITS> {
    type View = ArchivedPackedU8SliceView<'a, BITS>;

    const FIELD_ENCODING: FieldEncoding = FieldEncoding::PackedBits;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        let len = cursor.read_u32(context)? as usize;
        let byte_len = packed_byte_len(len, usize::from(BITS))?;
        let bytes = cursor.read_exact(byte_len, context)?;

        let max = if BITS == 8 {
            u8::MAX
        } else {
            (1u8 << BITS) - 1
        };
        for index in 0..len {
            let bit_offset = index * usize::from(BITS);
            let value = read_packed_bits(bytes, bit_offset, usize::from(BITS));
            if value > max {
                return Err(context.validation_error("Packed integer value out of range", pos));
            }
        }

        Ok(ArchivedPackedU8SliceView { len, bytes })
    }
}

impl<const BITS: u8> ArchivedDefault for ArchivedPackedU8SliceView<'_, BITS> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedPackedU8SliceView<'static, 8> =
            ArchivedPackedU8SliceView { len: 0, bytes: &[] };
        unsafe { &*(&DEFAULT as *const ArchivedPackedU8SliceView<'static, 8> as *const Self) }
    }
}

/// Borrowed packed sequence wrapper.
pub struct PackedSlice<'a, T, const BITS: u8> {
    pub(crate) values: &'a [T],
}

pub type PackedBoolSlice<'a> = PackedSlice<'a, bool, 1>;
pub type PackedU8Slice<'a, const BITS: u8> = PackedSlice<'a, u8, BITS>;

impl<'a> PackedSlice<'a, bool, 1> {
    pub fn new(values: &'a [bool]) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &'a [bool] {
        self.values
    }
}

impl<'a> From<&'a [bool]> for PackedSlice<'a, bool, 1> {
    fn from(values: &'a [bool]) -> Self {
        Self::new(values)
    }
}

impl<'a, const BITS: u8> PackedSlice<'a, u8, BITS> {
    pub fn new(values: &'a [u8]) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &'a [u8] {
        self.values
    }
}

impl<'a, const BITS: u8> From<&'a [u8]> for PackedSlice<'a, u8, BITS> {
    fn from(values: &'a [u8]) -> Self {
        Self::new(values)
    }
}

impl Archive for PackedSlice<'_, bool, 1> {
    type Archived = ArchivedPackedBoolSlice;
}

impl<const BITS: u8> Archive for PackedSlice<'_, u8, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
}

#[cfg(feature = "alloc")]
impl Restore<Vec<bool>> for ArchivedPackedBoolSliceView<'_> {
    fn restore(&self) -> Result<Vec<bool>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            out.push(self.get(i).unwrap());
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<const BITS: u8> Restore<Vec<u8>> for ArchivedPackedU8SliceView<'_, BITS> {
    fn restore(&self) -> Result<Vec<u8>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            out.push(self.get(i).unwrap());
        }
        Ok(out)
    }
}
