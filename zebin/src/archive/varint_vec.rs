use alloc::vec::Vec;
use core::task::Poll;

use crate::{
    archive_impl::varint::{VarIntNumber, decode_u64, encode_u64, encoded_len_u64},
    io::ForwardSequenceStrategy,
    prelude::*,
};

/// A compact vector of VarInts.
#[derive(Clone)]
pub struct VarIntVec<T> {
    pub values: Vec<T>,
}

impl<T> VarIntVec<T> {
    pub fn new(values: impl Into<VarIntVec<T>>) -> Self {
        values.into()
    }
}

impl<T> From<Vec<T>> for VarIntVec<T> {
    fn from(values: Vec<T>) -> Self {
        Self { values }
    }
}

impl<'a, T: 'a + Clone> From<&'a [T]> for VarIntVec<T> {
    fn from(values: &'a [T]) -> Self {
        Self { values: values.to_vec() }
    }
}

impl<T: Clone> From<&Vec<T>> for VarIntVec<T> {
    fn from(values: &Vec<T>) -> Self {
        Self { values: values.clone() }
    }
}

/// A compact slice of VarInts.
pub struct PackedVarIntSlice<'a, T> {
    pub values: &'a [T],
}

impl<'a, T> PackedVarIntSlice<'a, T> {
    pub fn new(values: &'a [T]) -> Self {
        Self { values }
    }
}

/// Decoded VarIntVec view. Also used as the zero-sized archive marker.
pub struct ArchivedVarIntVec<T> {
    values: Vec<T>,
}

impl<T: VarIntNumber> ArchivedVarIntVec<T> {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<T> {
        self.values.get(index).copied()
    }

    pub fn iter(&self) -> core::iter::Copied<core::slice::Iter<'_, T>> {
        self.values.iter().copied()
    }
}

impl<T> SchemaAware for ArchivedVarIntVec<T> {
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

impl<T: 'static> ArchivedDefault for ArchivedVarIntVec<T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedVarIntVec<()> = ArchivedVarIntVec { values: Vec::new() };
        unsafe { &*(&DEFAULT as *const ArchivedVarIntVec<()> as *const ArchivedVarIntVec<T>) }
    }
}

impl<T> ArchivedLayout for ArchivedVarIntVec<T> {
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::LengthPrefixed;
}

impl<'a, T: VarIntNumber + 'a> Decode<'a> for ArchivedVarIntVec<T> {
    type View = ArchivedVarIntVec<T>;
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        let mut values = Vec::with_capacity(len);
        for index in 0..len {
            let mut guard = context.push_index(index);
            values.push(decode_u64::<T, _>(cursor, &mut *guard)?);
        }
        Ok(ArchivedVarIntVec { values })
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let len = cursor.read_u32(context)? as usize;
        for index in 0..len {
            let mut guard = context.push_index(index);
            let _ = decode_u64::<T, _>(cursor, &mut *guard)?;
        }
        Ok(())
    }
}

impl<T: VarIntNumber> Archive for VarIntVec<T> {
    type Archived = ArchivedVarIntVec<T>;
}

impl<T> Archive for ArchivedVarIntVec<T> {
    type Archived = Self;
}

/// Owned-input encoder for `VarIntVec<T>`.
pub struct OwnedVarIntVecEncoder<T: VarIntNumber> {
    values: Vec<T>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    index: usize,
    cursor: usize,
    current_val_buf: [u8; 10],
    current_val_len: usize,
}

impl<T: VarIntNumber> OwnedVarIntVecEncoder<T> {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            len_prefix: [0; 4],
            prefix_cursor: 0,
            index: 0,
            cursor: 0,
            current_val_buf: [0u8; 10],
            current_val_len: 0,
        }
    }
}

impl<T: VarIntNumber> Default for OwnedVarIntVecEncoder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: VarIntNumber> Encoder for OwnedVarIntVecEncoder<T> {
    type Input = VarIntVec<T>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let len = item.values.len() as u32;
        self.values = item.values;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.index = 0;
        self.cursor = 0;
        self.current_val_len = 0;
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

        while self.index < self.values.len() {
            if self.current_val_len == 0 {
                let val = self.values[self.index].to_u64();
                self.current_val_len = encoded_len_u64(val);
                encode_u64(val, &mut self.current_val_buf[..self.current_val_len]);
                self.cursor = 0;
            }

            let remaining = self.current_val_len - self.cursor;
            if sink
                .write(&self.current_val_buf[self.cursor..self.current_val_len])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }

            self.current_val_len = 0;
            self.index += 1;
        }

        Ok(Poll::Ready(()))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<T: VarIntNumber> Encode for VarIntVec<T> {
    type Input<'a>
        = VarIntVec<T>
    where
        Self: 'a;
    type Encoder<'a>
        = OwnedVarIntVecEncoder<T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        OwnedVarIntVecEncoder::new()
    }
}

impl<T: VarIntNumber> Encode for ArchivedVarIntVec<T> {
    type Input<'a>
        = ArchivedVarIntVec<T>
    where
        Self: 'a;
    type Encoder<'a>
        = OwnedArchivedVarIntVecEncoder<T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        OwnedArchivedVarIntVecEncoder::new()
    }
}

/// Owned-input encoder for `ArchivedVarIntVec<T>`.
pub struct OwnedArchivedVarIntVecEncoder<T: VarIntNumber> {
    values: Vec<T>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    index: usize,
    cursor: usize,
    current_val_buf: [u8; 10],
    current_val_len: usize,
}

impl<T: VarIntNumber> OwnedArchivedVarIntVecEncoder<T> {
    pub fn new() -> Self {
        Self {
            values: Vec::new(),
            len_prefix: [0; 4],
            prefix_cursor: 0,
            index: 0,
            cursor: 0,
            current_val_buf: [0u8; 10],
            current_val_len: 0,
        }
    }
}

impl<T: VarIntNumber> Default for OwnedArchivedVarIntVecEncoder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: VarIntNumber> Encoder for OwnedArchivedVarIntVecEncoder<T> {
    type Input = ArchivedVarIntVec<T>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let len = item.values.len() as u32;
        self.values = item.values;
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.index = 0;
        self.cursor = 0;
        self.current_val_len = 0;
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

        while self.index < self.values.len() {
            if self.current_val_len == 0 {
                let val = self.values[self.index].to_u64();
                self.current_val_len = encoded_len_u64(val);
                encode_u64(val, &mut self.current_val_buf[..self.current_val_len]);
                self.cursor = 0;
            }

            let remaining = self.current_val_len - self.cursor;
            if sink
                .write(&self.current_val_buf[self.cursor..self.current_val_len])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }

            self.current_val_len = 0;
            self.index += 1;
        }

        Ok(Poll::Ready(()))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<'a, T: VarIntNumber> Archive for PackedVarIntSlice<'a, T> {
    type Archived = ArchivedVarIntVec<T>;
}

impl<'a, T: VarIntNumber> Encode for PackedVarIntSlice<'a, T> {
    type Input<'b>
        = PackedVarIntSlice<'a, T>
    where
        Self: 'b;
    type Encoder<'b>
        = PackedVarIntSliceEncoder<'a, T>
    where
        Self: 'b;

    fn encoder<'b>() -> Self::Encoder<'b>
    where
        Self: 'b,
    {
        PackedVarIntSliceEncoder::new()
    }
}

/// Owned-input encoder for `PackedVarIntSlice` (its borrow lives in `'a`).
pub struct PackedVarIntSliceEncoder<'a, T: VarIntNumber> {
    values: Option<&'a [T]>,
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    index: usize,
    cursor: usize,
    current_val_buf: [u8; 10],
    current_val_len: usize,
}

impl<'a, T: VarIntNumber> PackedVarIntSliceEncoder<'a, T> {
    pub fn new() -> Self {
        Self {
            values: None,
            len_prefix: [0; 4],
            prefix_cursor: 0,
            index: 0,
            cursor: 0,
            current_val_buf: [0u8; 10],
            current_val_len: 0,
        }
    }
}

impl<'a, T: VarIntNumber> Default for PackedVarIntSliceEncoder<'a, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T: VarIntNumber> Encoder for PackedVarIntSliceEncoder<'a, T> {
    type Input = PackedVarIntSlice<'a, T>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let values = item.values;
        let len = values.len() as u32;
        self.values = Some(values);
        self.len_prefix = len.to_le_bytes();
        self.prefix_cursor = 0;
        self.index = 0;
        self.cursor = 0;
        self.current_val_len = 0;
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

        let values = self
            .values
            .expect("PackedVarIntSliceEncoder polled before input");
        while self.index < values.len() {
            if self.current_val_len == 0 {
                let val = values[self.index].to_u64();
                self.current_val_len = encoded_len_u64(val);
                encode_u64(val, &mut self.current_val_buf[..self.current_val_len]);
                self.cursor = 0;
            }

            let remaining = self.current_val_len - self.cursor;
            if sink
                .write(&self.current_val_buf[self.cursor..self.current_val_len])?
                .advance_cursor(&mut self.cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }

            self.current_val_len = 0;
            self.index += 1;
        }

        Ok(Poll::Ready(()))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<T: VarIntNumber> MeasureBody for VarIntVec<T> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let mut total = 4usize;
        for v in &self.values {
            total = total
                .checked_add(encoded_len_u64(v.to_u64()))
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        Ok(total)
    }
}

impl<T: VarIntNumber> MeasureBody for ArchivedVarIntVec<T> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let mut total = 4usize;
        for v in &self.values {
            total = total
                .checked_add(encoded_len_u64(v.to_u64()))
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        Ok(total)
    }
}

impl<'a, T: VarIntNumber> MeasureBody for PackedVarIntSlice<'a, T> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let mut total = 4usize;
        for v in self.values {
            total = total
                .checked_add(encoded_len_u64(v.to_u64()))
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?;
        }
        Ok(total)
    }
}

impl<T: VarIntNumber> Restore<Vec<T>> for ArchivedVarIntVec<T> {
    fn restore(&self) -> Result<Vec<T>, ZebinError> {
        Ok(self.values.clone())
    }
}

impl<T: VarIntNumber> Restore<VarIntVec<T>> for ArchivedVarIntVec<T> {
    fn restore(&self) -> Result<VarIntVec<T>, ZebinError> {
        Ok(VarIntVec {
            values: self.values.clone(),
        })
    }
}

impl<T, U> Restore<VarIntVec<U>> for VarIntVec<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VarIntVec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.values.len());
        for item in &self.values {
            out.push(item.restore()?);
        }
        Ok(VarIntVec { values: out })
    }
}

impl<'a, T, U> Restore<VarIntVec<U>> for PackedVarIntSlice<'a, T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<VarIntVec<U>, ZebinError> {
        let mut out = Vec::with_capacity(self.values.len());
        for item in self.values {
            out.push(item.restore()?);
        }
        Ok(VarIntVec { values: out })
    }
}
