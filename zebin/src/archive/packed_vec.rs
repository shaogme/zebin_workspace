use alloc::{vec, vec::Vec};
use core::task::Poll;

use crate::{
    archive::packed::{ArchivedPackedBoolSlice, ArchivedPackedU8Slice},
    core::rel_ptr::RelPtr,
    error::{ArchiveError, ValidateError, ZebinError},
    io::sink::{ByteSink, LayoutSink},
    traits::{Archive, Serialize, SerializeState},
    utils::num::usize_to_u32,
};

fn packed_byte_len(value_count: usize, bits_per_value: usize) -> Result<usize, ValidateError> {
    let total_bits =
        value_count
            .checked_mul(bits_per_value)
            .ok_or(ValidateError::ValidationError {
                message: "Packed length calculation overflow",
                pos: 0,
                path: Default::default(),
            })?;
    Ok(total_bits.div_ceil(8))
}

pub(crate) fn pack_small_values(
    values: &[u8],
    bits_per_value: usize,
) -> Result<Vec<u8>, ZebinError> {
    if bits_per_value == 0 || bits_per_value > 8 {
        return Err(ZebinError::SerializationError {
            pos: 0,
            message: "Packed bits must be between 1 and 8",
        });
    }

    let out_len = packed_byte_len(values.len(), bits_per_value).map_err(|_| {
        ZebinError::SerializationError {
            pos: 0,
            message: "Packed length calculation overflow",
        }
    })?;
    let mut out = vec![0u8; out_len];
    let mask = if bits_per_value == 8 {
        u8::MAX
    } else {
        (1u8 << bits_per_value) - 1
    };

    for (index, &value) in values.iter().enumerate() {
        if value > mask {
            return Err(ZebinError::SerializationError {
                pos: index,
                message: "Value exceeds packed bit capacity",
            });
        }

        let bit_offset = index
            .checked_mul(bits_per_value)
            .ok_or(ZebinError::ArithmeticOverflow { pos: index })?;
        let byte_index = bit_offset / 8;
        let bit_shift = bit_offset % 8;
        out[byte_index] |= value << bit_shift;
        if bit_shift + bits_per_value > 8 {
            out[byte_index + 1] |= value >> (8 - bit_shift);
        }
    }

    Ok(out)
}

pub(crate) fn pack_bools(values: &[bool]) -> Vec<u8> {
    let out_len = packed_byte_len(values.len(), 1).expect("bool packing length should fit");
    let mut out = vec![0u8; out_len];

    #[cfg(feature = "simd")]
    {
        use wide::u8x16;
        let mut out_cursor = 0usize;
        let mut chunks = values.chunks_exact(16);
        for chunk in &mut chunks {
            let mut lanes = [0u8; 16];
            for (index, value) in chunk.iter().enumerate() {
                lanes[index] = *value as u8;
            }

            let mask = u8x16::from(lanes).simd_gt(u8x16::splat(0)).to_bitmask();
            out[out_cursor] = (mask & 0x00FF) as u8;
            if out_cursor + 1 < out.len() {
                out[out_cursor + 1] = ((mask >> 8) & 0x00FF) as u8;
            }
            out_cursor += 2;
        }

        let remainder = chunks.remainder();
        for (index, value) in remainder.iter().enumerate() {
            if *value {
                let bit = index;
                let byte_index = out_cursor + (bit / 8);
                let bit_in_byte = bit % 8;
                out[byte_index] |= 1 << bit_in_byte;
            }
        }
    }

    #[cfg(not(feature = "simd"))]
    {
        for (index, &value) in values.iter().enumerate() {
            if value {
                let byte_index = index / 8;
                let bit_in_byte = index % 8;
                out[byte_index] |= 1 << bit_in_byte;
            }
        }
    }

    out
}

/// Packed sequence state shared by the packed APIs.
#[doc(hidden)]
pub struct PackedSequenceState {
    bytes: Vec<u8>,
    cursor: usize,
    start_pos: Option<usize>,
}

impl PackedSequenceState {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            cursor: 0,
            start_pos: None,
        }
    }
}

impl<'a> SerializeState<'a> for PackedSequenceState {
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        if self.start_pos.is_none() {
            self.start_pos = Some(encoder.pos());
        }

        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < self.bytes.len() {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(
                self.start_pos.expect("start position set above"),
            ))
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
        = PackedSequenceState
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_bools(self.values.as_slice())))
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
        = PackedSequenceState
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_small_values(
            self.values.as_slice(),
            usize::from(BITS),
        )?))
    }
}

impl Serialize for crate::archive::packed::PackedSlice<'_, bool, 1> {
    type State<'a>
        = PackedSequenceState
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_bools(self.values())))
    }
}

impl<const BITS: u8> Serialize for crate::archive::packed::PackedSlice<'_, u8, BITS> {
    type State<'a>
        = PackedSequenceState
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_small_values(
            self.values(),
            usize::from(BITS),
        )?))
    }
}
