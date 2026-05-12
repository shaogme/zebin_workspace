use alloc::vec::Vec;
use core::task::Poll;

use crate::{
    archive::packed::{ArchivedPackedBoolSlice, ArchivedPackedU8Slice},
    error::ZebinError,
    traits::{Archive, ByteSink, Encode, EncodeState, Restore},
};

enum PackedData<'a> {
    Bool(&'a [bool]),
    U8(&'a [u8]),
}

/// Packed sequence state shared by the packed APIs.
#[doc(hidden)]
pub struct PackedSequenceState<'a> {
    data: PackedData<'a>,
    bits_per_value: u8,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    index: usize,
    len: usize,
    buf: [u8; 64],
    buf_len: usize,
    buf_cursor: usize,
}

impl<'a> PackedSequenceState<'a> {
    pub fn new_bool(values: &'a [bool]) -> Result<Self, ZebinError> {
        Self::new(PackedData::Bool(values), 1, values.len())
    }

    pub fn new_u8(values: &'a [u8], bits_per_value: u8) -> Result<Self, ZebinError> {
        Self::new(PackedData::U8(values), bits_per_value, values.len())
    }

    fn new(data: PackedData<'a>, bits_per_value: u8, len: usize) -> Result<Self, ZebinError> {
        let len_u32 = u32::try_from(len).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "Packed length exceeds u32 range",
        })?;
        Ok(Self {
            data,
            bits_per_value,
            len_prefix: len_u32.to_le_bytes(),
            prefix_cursor: 0,
            index: 0,
            len,
            buf: [0u8; 64],
            buf_len: 0,
            buf_cursor: 0,
        })
    }

    fn fill_buf(&mut self) -> Result<(), ZebinError> {
        self.buf.fill(0);
        let mut bit_offset = 0usize;
        let bits_per_value = self.bits_per_value as usize;

        while self.index < self.len && bit_offset + bits_per_value <= 64 * 8 {
            match self.data {
                PackedData::Bool(values) => {
                    if values[self.index] {
                        let byte_idx = bit_offset / 8;
                        let bit_idx = bit_offset % 8;
                        self.buf[byte_idx] |= 1 << bit_idx;
                    }
                }
                PackedData::U8(values) => {
                    let value = values[self.index];
                    let mask = if bits_per_value == 8 {
                        u8::MAX
                    } else {
                        (1u8 << bits_per_value) - 1
                    };
                    if value > mask {
                        return Err(ZebinError::SerializationError {
                            pos: self.index,
                            message: "Value exceeds packed bit capacity",
                        });
                    }
                    let byte_idx = bit_offset / 8;
                    let bit_shift = bit_offset % 8;
                    self.buf[byte_idx] |= value << bit_shift;
                    if bit_shift + bits_per_value > 8 {
                        self.buf[byte_idx + 1] |= value >> (8 - bit_shift);
                    }
                }
            }
            bit_offset += bits_per_value;
            self.index += 1;
        }

        self.buf_len = bit_offset.div_ceil(8);
        self.buf_cursor = 0;
        Ok(())
    }
}

impl<'a> EncodeState<'a> for PackedSequenceState<'a> {
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

        loop {
            if self.buf_cursor >= self.buf_len {
                if self.index >= self.len {
                    return Ok(Poll::Ready(()));
                }
                self.fill_buf()?;
            }

            let remaining = self.buf_len - self.buf_cursor;
            if encoder
                .write(&self.buf[self.buf_cursor..self.buf_len])?
                .advance_cursor(&mut self.buf_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }
    }
}

/// Owned packed sequence wrapper.
pub struct PackedVec<T, const BITS: u8> {
    values: Vec<T>,
}

pub type PackedBoolVec = PackedVec<bool, 1>;
pub type PackedU8Vec<const BITS: u8> = PackedVec<u8, BITS>;

impl<T, const BITS: u8> PackedVec<T, BITS> {
    pub fn new(values: Vec<T>) -> Self {
        Self { values }
    }

    pub fn values(&self) -> &[T] {
        self.values.as_slice()
    }

    pub fn into_inner(self) -> Vec<T> {
        self.values
    }
}

impl<T, const BITS: u8> From<Vec<T>> for PackedVec<T, BITS> {
    fn from(values: Vec<T>) -> Self {
        Self::new(values)
    }
}

impl<T, const BITS: u8> core::iter::FromIterator<T> for PackedVec<T, BITS> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

impl<T, const BITS: u8> IntoIterator for PackedVec<T, BITS> {
    type Item = T;
    type IntoIter = alloc::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<'a, T, const BITS: u8> IntoIterator for &'a PackedVec<T, BITS> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl Archive for PackedVec<bool, 1> {
    type Archived = ArchivedPackedBoolSlice;
}

impl Encode for PackedVec<bool, 1> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        PackedSequenceState::new_bool(self.values.as_slice())
    }
}

impl<const BITS: u8> Archive for PackedVec<u8, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
}

impl<const BITS: u8> Encode for PackedVec<u8, BITS> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        PackedSequenceState::new_u8(self.values.as_slice(), BITS)
    }
}

impl Encode for crate::archive::packed::PackedSlice<'_, bool, 1> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        PackedSequenceState::new_bool(self.values())
    }
}

impl<const BITS: u8> Encode for crate::archive::packed::PackedSlice<'_, u8, BITS> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        PackedSequenceState::new_u8(self.values(), BITS)
    }
}

impl<T, const BITS: u8, U> Restore<PackedVec<U, BITS>> for T
where
    T: Restore<Vec<U>>,
{
    fn restore(&self) -> Result<PackedVec<U, BITS>, ZebinError> {
        Ok(PackedVec::new(self.restore()?))
    }
}
