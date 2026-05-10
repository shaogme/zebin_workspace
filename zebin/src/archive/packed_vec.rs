use alloc::vec::Vec;
use core::task::Poll;

use crate::{
    archive::packed::{ArchivedPackedBoolSlice, ArchivedPackedU8Slice},
    core::rel_ptr::RelPtr,
    error::{ArchiveError, ZebinError},
    io::sink::{ByteSink, LayoutSink},
    traits::{Archive, Serialize, SerializeState},
    utils::num::usize_to_u32,
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
    index: usize,
    len: usize,
    start_pos: Option<usize>,
    buf: [u8; 64],
    buf_len: usize,
    buf_cursor: usize,
}

impl<'a> PackedSequenceState<'a> {
    pub fn new_bool(values: &'a [bool]) -> Self {
        Self {
            data: PackedData::Bool(values),
            bits_per_value: 1,
            index: 0,
            len: values.len(),
            start_pos: None,
            buf: [0u8; 64],
            buf_len: 0,
            buf_cursor: 0,
        }
    }

    pub fn new_u8(values: &'a [u8], bits_per_value: u8) -> Self {
        Self {
            data: PackedData::U8(values),
            bits_per_value,
            index: 0,
            len: values.len(),
            start_pos: None,
            buf: [0u8; 64],
            buf_len: 0,
            buf_cursor: 0,
        }
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

impl<'a> SerializeState<'a> for PackedSequenceState<'a> {
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if self.start_pos.is_none() {
            self.start_pos = Some(encoder.pos());
        }

        loop {
            if self.buf_cursor >= self.buf_len {
                if self.index >= self.len {
                    return Ok(Poll::Ready(
                        self.start_pos.expect("start position set above"),
                    ));
                }
                self.fill_buf()?;
            }

            let written = encoder.write(&self.buf[self.buf_cursor..self.buf_len])?;
            self.buf_cursor += written;
            if self.buf_cursor < self.buf_len {
                return Ok(Poll::Pending);
            }
        }
    }
}

/// Owned packed sequence wrapper.
pub struct PackedVec<T, const BITS: u8> {
    values: Vec<T>,
}

/// Owned bit-packed booleans.
pub type PackedBoolVec = PackedVec<bool, 1>;
/// Owned packed small integers.
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
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        let ptr = if self.values.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedPackedBoolSlice {
            ptr,
            len: usize_to_u32(self.values.len(), || ArchiveError::LengthOverflow {
                pos: archive_pos,
            })?,
        })
    }
}

impl Serialize for PackedVec<bool, 1> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new_bool(self.values.as_slice()))
    }
}

impl<const BITS: u8> Archive for PackedVec<u8, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        let ptr = if self.values.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedPackedU8Slice {
            ptr,
            len: usize_to_u32(self.values.len(), || ArchiveError::LengthOverflow {
                pos: archive_pos,
            })?,
        })
    }
}

impl<const BITS: u8> Serialize for PackedVec<u8, BITS> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new_u8(self.values.as_slice(), BITS))
    }
}

impl Serialize for crate::archive::packed::PackedSlice<'_, bool, 1> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new_bool(self.values()))
    }
}

impl<const BITS: u8> Serialize for crate::archive::packed::PackedSlice<'_, u8, BITS> {
    type State<'a>
        = PackedSequenceState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new_u8(self.values(), BITS))
    }
}
