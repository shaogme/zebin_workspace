use alloc::vec::Vec;
use core::task::Poll;

use crate::{
    archive::varint::{VarIntNumber, decode_u64, encode_u64, encoded_len_u64},
    core::schema::{FieldEncoding, ObjectEncoding},
    error::{DecodeError, ZebinError},
    read::Cursor,
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Restore, SchemaAware,
        Serialize, SerializeState,
    },
    validation::context::ValidationContext,
};

/// A compact vector of VarInts.
pub struct VarIntVec<T> {
    pub values: Vec<T>,
}

impl<T> VarIntVec<T> {
    pub fn new(values: Vec<T>) -> Self {
        Self { values }
    }
}

impl<T> From<Vec<T>> for VarIntVec<T> {
    fn from(values: Vec<T>) -> Self {
        Self::new(values)
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

pub struct VarIntVecBuilderState<'a, T: VarIntNumber> {
    values: &'a [T],
    len_prefix: [u8; 4],
    prefix_cursor: usize,
    index: usize,
    cursor: usize,
    current_val_buf: [u8; 10],
    current_val_len: usize,
}

impl<'a, T: VarIntNumber> VarIntVecBuilderState<'a, T> {
    pub(crate) fn new(values: &'a [T]) -> Result<Self, ZebinError> {
        let len = u32::try_from(values.len()).map_err(|_| ZebinError::SerializationError {
            pos: 0,
            message: "VarIntVec length exceeds u32 range",
        })?;
        Ok(Self {
            values,
            len_prefix: len.to_le_bytes(),
            prefix_cursor: 0,
            index: 0,
            cursor: 0,
            current_val_buf: [0u8; 10],
            current_val_len: 0,
        })
    }
}

impl<'a, T: VarIntNumber> SerializeState<'a> for VarIntVecBuilderState<'a, T> {
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.len_prefix.len() {
            let written = encoder.write(&self.len_prefix[self.prefix_cursor..])?;
            self.prefix_cursor += written;
            if self.prefix_cursor < self.len_prefix.len() {
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

            let written =
                encoder.write(&self.current_val_buf[self.cursor..self.current_val_len])?;
            self.cursor += written;
            if self.cursor < self.current_val_len {
                return Ok(Poll::Pending);
            }

            self.current_val_len = 0;
            self.index += 1;
        }

        Ok(Poll::Ready(()))
    }
}

impl<T: VarIntNumber> Serialize for VarIntVec<T> {
    type State<'a>
        = VarIntVecBuilderState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        VarIntVecBuilderState::new(&self.values)
    }
}

impl<T: VarIntNumber> Serialize for ArchivedVarIntVec<T> {
    type State<'a>
        = VarIntVecBuilderState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        VarIntVecBuilderState::new(&self.values)
    }
}

impl<'a, T: VarIntNumber> Archive for PackedVarIntSlice<'a, T> {
    type Archived = ArchivedVarIntVec<T>;
}

impl<'a, T: VarIntNumber> Serialize for PackedVarIntSlice<'a, T> {
    type State<'b>
        = VarIntVecBuilderState<'b, T>
    where
        Self: 'b;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        VarIntVecBuilderState::new(self.values)
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
