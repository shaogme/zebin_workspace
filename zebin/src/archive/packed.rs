use crate::{
    core::schema::{FieldEncoding, ObjectEncoding},
    error::{DecodeError, ZebinError},
    read::Cursor,
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Encode, Encoder,
        SchemaAware,
    },
    validation::context::ValidationContext,
};
use core::task::Poll;

impl SchemaAware for ArchivedPackedBoolSliceView<'_> {
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

impl Archive for ArchivedPackedBoolSliceView<'_> {
    type Archived = ArchivedPackedBoolSlice;
}

impl<const BITS: u8> SchemaAware for ArchivedPackedU8SliceView<'_, BITS> {
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

impl<const BITS: u8> Archive for ArchivedPackedU8SliceView<'_, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
}

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

pub(crate) fn packed_byte_len(
    value_count: usize,
    bits_per_value: usize,
) -> Result<usize, DecodeError> {
    let total_bits =
        value_count
            .checked_mul(bits_per_value)
            .ok_or(DecodeError::ValidationError {
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

impl Archive for ArchivedPackedBoolSlice {
    type Archived = Self;
}

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

impl ArchivedLayout for ArchivedPackedBoolSlice {
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Packed;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::PackedBits;
}

impl<'a> Decode<'a> for ArchivedPackedBoolSlice {
    type View = ArchivedPackedBoolSliceView<'a>;
    #[cfg(feature = "alloc")]
    type DecodeStrategy = crate::traits::ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let byte_len = packed_byte_len(len, 1)?;
        let bytes = cursor.read_exact(byte_len, context)?;
        Ok(ArchivedPackedBoolSliceView { len, bytes })
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let byte_len = packed_byte_len(len, 1)?;
        let _ = cursor.read_exact(byte_len, context)?;
        Ok(())
    }
}

impl ArchivedDefault for ArchivedPackedBoolSliceView<'_> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedPackedBoolSliceView<'static> =
            ArchivedPackedBoolSliceView { len: 0, bytes: &[] };
        &DEFAULT
    }
}

/// Zero-sized decode marker for archived packed u8 slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchivedPackedU8Slice<const BITS: u8 = 8>;

impl<const BITS: u8> Archive for ArchivedPackedU8Slice<BITS> {
    type Archived = Self;
}

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

impl<const BITS: u8> ArchivedLayout for ArchivedPackedU8Slice<BITS> {
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Packed;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::PackedBits;
}

impl<'a, const BITS: u8> Decode<'a> for ArchivedPackedU8Slice<BITS> {
    type View = ArchivedPackedU8SliceView<'a, BITS>;
    #[cfg(feature = "alloc")]
    type DecodeStrategy = crate::traits::ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
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

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
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

        Ok(())
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

impl<'a> Encode for ArchivedPackedBoolSliceView<'a> {
    type Encoder<'b>
        = PackedViewEncoder<'b, ArchivedPackedBoolSliceView<'a>>
    where
        Self: 'b;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(PackedViewEncoder::new(self.len, self.bytes))
    }
}

impl<'a, const BITS: u8> Encode for ArchivedPackedU8SliceView<'a, BITS> {
    type Encoder<'b>
        = PackedViewEncoder<'b, ArchivedPackedU8SliceView<'a, BITS>>
    where
        Self: 'b;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(PackedViewEncoder::new(self.len, self.bytes))
    }
}

/// State for serializing an already-packed view.
pub struct PackedViewEncoder<'a, I = ()> {
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    bytes: &'a [u8],
    bytes_cursor: usize,
    _phantom: core::marker::PhantomData<&'a I>,
}

impl<'a, I> PackedViewEncoder<'a, I> {
    pub fn new(len: usize, bytes: &'a [u8]) -> Self {
        Self {
            len_prefix: (len as u32).to_le_bytes(),
            prefix_cursor: 0,
            bytes,
            bytes_cursor: 0,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, I> Encoder<'a> for PackedViewEncoder<'a, I> {
    type Input = &'a I;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < 4 {
            let remaining = 4 - self.prefix_cursor;
            if sink
                .write(&self.len_prefix[self.prefix_cursor..])?
                .advance_cursor(&mut self.prefix_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

        if self.bytes_cursor < self.bytes.len() {
            let remaining = self.bytes.len() - self.bytes_cursor;
            if sink
                .write(&self.bytes[self.bytes_cursor..])?
                .advance_cursor(&mut self.bytes_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

        Ok(Poll::Ready(()))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

#[cfg(feature = "alloc")]
impl crate::traits::Restore<Vec<bool>> for ArchivedPackedBoolSliceView<'_> {
    fn restore(&self) -> Result<Vec<bool>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            out.push(self.get(i).unwrap());
        }
        Ok(out)
    }
}

#[cfg(feature = "alloc")]
impl<const BITS: u8> crate::traits::Restore<Vec<u8>> for ArchivedPackedU8SliceView<'_, BITS> {
    fn restore(&self) -> Result<Vec<u8>, ZebinError> {
        let mut out = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            out.push(self.get(i).unwrap());
        }
        Ok(out)
    }
}
