use alloc::vec::Vec;
use core::task::Poll;

use crate::prelude::*;

pub enum PackedData<'a> {
    Empty,
    Bool(&'a [bool]),
    U8(&'a [u8]),
}

pub trait ToPackedData<'a> {
    fn to_packed_data(self) -> (PackedData<'a>, u8);
}

/// Packed sequence encoder shared by the packed APIs.
#[doc(hidden)]
pub struct PackedSequenceEncoder<'a, I = ()> {
    data: PackedData<'a>,
    bits_per_value: u8,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    index: usize,
    len: usize,
    buf: [u8; 64],
    buf_len: usize,
    buf_cursor: usize,
    _phantom: core::marker::PhantomData<&'a I>,
}

impl<'a, I> PackedSequenceEncoder<'a, I> {
    pub fn new_bool(values: &'a [bool]) -> Self {
        Self::new(PackedData::Bool(values), 1, values.len())
    }

    pub fn new_u8(values: &'a [u8], bits_per_value: u8) -> Self {
        Self::new(PackedData::U8(values), bits_per_value, values.len())
    }

    pub fn new_empty() -> Self {
        Self {
            data: PackedData::Empty,
            bits_per_value: 0,
            len_prefix: [0; 4],
            prefix_cursor: 0,
            index: 0,
            len: 0,
            buf: [0u8; 64],
            buf_len: 0,
            buf_cursor: 0,
            _phantom: core::marker::PhantomData,
        }
    }

    fn new(data: PackedData<'a>, bits_per_value: u8, len: usize) -> Self {
        let len_u32 = len as u32;
        Self {
            data,
            bits_per_value,
            len_prefix: len_u32.to_le_bytes(),
            prefix_cursor: 0,
            index: 0,
            len,
            buf: [0u8; 64],
            buf_len: 0,
            buf_cursor: 0,
            _phantom: core::marker::PhantomData,
        }
    }

    fn fill_buf(&mut self) -> Result<(), ZebinError> {
        self.buf.fill(0);
        let mut bit_offset = 0usize;
        let bits_per_value = self.bits_per_value as usize;

        while self.index < self.len && bit_offset + bits_per_value <= 64 * 8 {
            match self.data {
                PackedData::Empty => {}
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

impl<'a, I> Encoder for PackedSequenceEncoder<'a, I>
where
    I: ToPackedData<'a>,
{
    type Input = I;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let (data, bits) = item.to_packed_data();
        let len = match data {
            PackedData::Empty => 0,
            PackedData::Bool(v) => v.len(),
            PackedData::U8(v) => v.len(),
        };
        let len_u32 = len as u32;
        self.data = data;
        self.bits_per_value = bits;
        self.len = len;
        self.len_prefix = len_u32.to_le_bytes();
        self.prefix_cursor = 0;
        self.index = 0;
        self.buf_len = 0;
        self.buf_cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
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

        loop {
            if self.buf_cursor >= self.buf_len {
                if self.index >= self.len {
                    return Ok(Poll::Ready(()));
                }
                self.fill_buf()?;
            }

            let remaining = self.buf_len - self.buf_cursor;
            if sink
                .write(&self.buf[self.buf_cursor..self.buf_len])?
                .advance_cursor(&mut self.buf_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }
    }

    fn finish<S: StorageMut + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

/// Owned packed sequence wrapper.
#[derive(Clone)]
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

impl<T: Clone, const BITS: u8> From<&Vec<T>> for PackedVec<T, BITS> {
    fn from(values: &Vec<T>) -> Self {
        Self::new(values.clone())
    }
}

impl<T: Clone, const BITS: u8> From<&[T]> for PackedVec<T, BITS> {
    fn from(values: &[T]) -> Self {
        Self::new(values.to_vec())
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

impl<'a> ToPackedData<'a> for &'a PackedVec<bool, 1> {
    fn to_packed_data(self) -> (PackedData<'a>, u8) {
        (PackedData::Bool(self.values.as_slice()), 1)
    }
}

impl<'a, const BITS: u8> ToPackedData<'a> for &'a PackedVec<u8, BITS> {
    fn to_packed_data(self) -> (PackedData<'a>, u8) {
        (PackedData::U8(self.values.as_slice()), BITS)
    }
}

impl<'a, 'b> ToPackedData<'a> for PackedSlice<'b, bool, 1>
where
    'b: 'a,
{
    fn to_packed_data(self) -> (PackedData<'a>, u8) {
        (PackedData::Bool(self.values()), 1)
    }
}

impl<'a, 'b, const BITS: u8> ToPackedData<'a> for PackedSlice<'b, u8, BITS>
where
    'b: 'a,
{
    fn to_packed_data(self) -> (PackedData<'a>, u8) {
        (PackedData::U8(self.values()), BITS)
    }
}

impl<'a, 'b> ToPackedData<'a> for &'a PackedSlice<'b, bool, 1> {
    fn to_packed_data(self) -> (PackedData<'a>, u8) {
        (PackedData::Bool(self.values()), 1)
    }
}

impl<'a, 'b, const BITS: u8> ToPackedData<'a> for &'a PackedSlice<'b, u8, BITS> {
    fn to_packed_data(self) -> (PackedData<'a>, u8) {
        (PackedData::U8(self.values()), BITS)
    }
}

impl Archive for PackedVec<bool, 1> {
    type Archived = ArchivedPackedBoolSlice;
}

/// Owned-input encoder for `PackedVec<bool, 1>`.
///
/// Holds the vector while streaming; the inner `PackedSequenceEncoder`
/// borrows from it via a self-referential pattern made safe by pinning the
/// owner inside this struct.
pub struct PackedBoolVecEncoder {
    owner: Option<PackedVec<bool, 1>>,
    bits: alloc::vec::Vec<u8>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    cursor: usize,
}

impl PackedBoolVecEncoder {
    pub fn new() -> Self {
        Self {
            owner: None,
            bits: alloc::vec::Vec::new(),
            len_prefix: [0; 4],
            prefix_cursor: 0,
            cursor: 0,
        }
    }
}

impl Default for PackedBoolVecEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder for PackedBoolVecEncoder {
    type Input = PackedVec<bool, 1>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let len = item.values().len() as u32;
        // Eagerly pack into a Vec<u8>. Avoids the self-referential lifetime
        // issue at the cost of one materialization pass.
        let byte_len = (item.values().len()).div_ceil(8);
        let mut bits = vec![0; byte_len];
        for (i, b) in item.values().iter().enumerate() {
            if *b {
                bits[i / 8] |= 1 << (i % 8);
            }
        }
        self.owner = Some(item);
        self.bits = bits;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
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
        let remaining = self.bits.len() - self.cursor;
        Ok(sink
            .write(&self.bits[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish<S: StorageMut + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl Encode for PackedVec<bool, 1> {
    type Input<'a>
        = PackedVec<bool, 1>
    where
        Self: 'a;
    type Encoder<'a>
        = PackedBoolVecEncoder
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        PackedBoolVecEncoder::new()
    }
}

impl<const BITS: u8> Archive for PackedVec<u8, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
}

/// Owned-input encoder for `PackedVec<u8, BITS>`.
pub struct PackedU8VecEncoder<const BITS: u8> {
    owner: Option<PackedVec<u8, BITS>>,
    bits: alloc::vec::Vec<u8>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    cursor: usize,
}

impl<const BITS: u8> PackedU8VecEncoder<BITS> {
    pub fn new() -> Self {
        Self {
            owner: None,
            bits: alloc::vec::Vec::new(),
            len_prefix: [0; 4],
            prefix_cursor: 0,
            cursor: 0,
        }
    }
}

impl<const BITS: u8> Default for PackedU8VecEncoder<BITS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BITS: u8> Encoder for PackedU8VecEncoder<BITS> {
    type Input = PackedVec<u8, BITS>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let len = item.values().len() as u32;
        let bits_per_value = BITS as usize;
        let total_bits = item
            .values()
            .len()
            .checked_mul(bits_per_value)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        let byte_len = total_bits.div_ceil(8);
        let mut bits = vec![0; byte_len];
        let mask = if bits_per_value == 8 {
            u8::MAX
        } else {
            (1u8 << bits_per_value) - 1
        };
        for (i, value) in item.values().iter().enumerate() {
            let value = *value;
            if value > mask {
                return Err(ZebinError::SerializationError {
                    pos: i,
                    message: "Value exceeds packed bit capacity",
                });
            }
            let bit_offset = i * bits_per_value;
            let byte_idx = bit_offset / 8;
            let bit_shift = bit_offset % 8;
            bits[byte_idx] |= value << bit_shift;
            if bit_shift + bits_per_value > 8 {
                bits[byte_idx + 1] |= value >> (8 - bit_shift);
            }
        }
        self.owner = Some(item);
        self.bits = bits;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
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
        let remaining = self.bits.len() - self.cursor;
        Ok(sink
            .write(&self.bits[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish<S: StorageMut + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<const BITS: u8> Encode for PackedVec<u8, BITS> {
    type Input<'a>
        = PackedVec<u8, BITS>
    where
        Self: 'a;
    type Encoder<'a>
        = PackedU8VecEncoder<BITS>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        PackedU8VecEncoder::new()
    }
}

impl<'b> Encode for PackedSlice<'b, bool, 1> {
    type Input<'a>
        = PackedSlice<'b, bool, 1>
    where
        Self: 'a;
    type Encoder<'a>
        = PackedSequenceEncoder<'a, PackedSlice<'b, bool, 1>>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        PackedSequenceEncoder::new_empty()
    }
}

impl<'b, const BITS: u8> Encode for PackedSlice<'b, u8, BITS> {
    type Input<'a>
        = PackedSlice<'b, u8, BITS>
    where
        Self: 'a;
    type Encoder<'a>
        = PackedSequenceEncoder<'a, PackedSlice<'b, u8, BITS>>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        PackedSequenceEncoder::new_empty()
    }
}

impl MeasureBody for PackedVec<bool, 1> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        Ok(4 + self.values().len().div_ceil(8))
    }
}

impl<const BITS: u8> MeasureBody for PackedVec<u8, BITS> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let bits = (self.values().len())
            .checked_mul(usize::from(BITS))
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        Ok(4 + bits.div_ceil(8))
    }
}

impl<'b> MeasureBody for PackedSlice<'b, bool, 1> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        Ok(4 + self.values().len().div_ceil(8))
    }
}

impl<'b, const BITS: u8> MeasureBody for PackedSlice<'b, u8, BITS> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let bits = (self.values().len())
            .checked_mul(usize::from(BITS))
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        Ok(4 + bits.div_ceil(8))
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
