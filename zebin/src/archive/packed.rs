use alloc::{string::ToString, vec::Vec};
use core::{num::NonZeroUsize, task::Poll};

use wide::u8x16;

use crate::{
    ZebinError, byteops,
    core::rel_ptr::RelPtr,
    num::{u32_to_usize, usize_to_u32},
    traits::{
        Archive, ArchiveBuilder, ArchiveState, ArchivedDecode, ArchivedLayout, ArchivedValidate,
        ArchivedValidationContext, ByteSink, LayoutSink,
    },
};

fn packed_byte_len(value_count: usize, bits_per_value: usize) -> Result<usize, ZebinError> {
    let total_bits = value_count
        .checked_mul(bits_per_value)
        .ok_or(ZebinError::WriteError)?;
    Ok(total_bits.div_ceil(8))
}

fn read_packed_bits(bytes: &[u8], bit_offset: usize, bits_per_value: usize) -> u8 {
    let mut value = 0u8;
    for bit in 0..bits_per_value {
        let absolute_bit = bit_offset + bit;
        let byte = bytes[absolute_bit / 8];
        let bit_value = (byte >> (absolute_bit % 8)) & 1;
        value |= bit_value << bit;
    }
    value
}

fn pack_small_values(values: &[u8], bits_per_value: usize) -> Result<Vec<u8>, ZebinError> {
    if bits_per_value == 0 || bits_per_value > 8 {
        return Err(ZebinError::WriteError);
    }

    let out_len = packed_byte_len(values.len(), bits_per_value)?;
    let mut out = vec![0u8; out_len];
    let mask = if bits_per_value == 8 {
        u8::MAX
    } else {
        (1u8 << bits_per_value) - 1
    };

    for (index, &value) in values.iter().enumerate() {
        if value > mask {
            return Err(ZebinError::WriteError);
        }

        let bit_offset = index
            .checked_mul(bits_per_value)
            .ok_or(ZebinError::WriteError)?;
        let byte_index = bit_offset / 8;
        let bit_shift = bit_offset % 8;
        out[byte_index] |= value << bit_shift;
        if bit_shift + bits_per_value > 8 {
            out[byte_index + 1] |= value >> (8 - bit_shift);
        }
    }

    Ok(out)
}

fn pack_bools(values: &[bool]) -> Vec<u8> {
    let out_len = packed_byte_len(values.len(), 1).expect("bool packing length should fit");
    let mut out = vec![0u8; out_len];

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

    out
}

/// Archived packed boolean slice. Values are stored as one bit per item.
#[repr(C)]
pub struct ArchivedPackedBoolSlice {
    ptr: Option<RelPtr<u8>>,
    len: u32,
}

impl ArchivedPackedBoolSlice {
    pub fn len(&self) -> usize {
        u32_to_usize(self.len, || ZebinError::ValidationError {
            message: "Archived packed bool length exceeds usize range".to_string(),
            pos: self as *const _ as usize,
        })
        .expect("validated packed bool length should fit in usize")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<bool> {
        let len = self.len();
        if index >= len {
            return None;
        }
        let bytes = unsafe { self.packed_bytes() };
        let byte_index = index / 8;
        let bit = index % 8;
        let value = (bytes.get(byte_index)? >> bit) & 1;
        Some(value != 0)
    }

    unsafe fn packed_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        let len = self.len();
        let byte_len = packed_byte_len(len, 1).expect("packed bool length should fit");
        let ptr = self
            .ptr
            .as_ref()
            .expect("non-empty packed bool slice must have a pointer");
        unsafe { core::slice::from_raw_parts(ptr.as_ptr(), byte_len) }
    }
}

impl ArchivedLayout for ArchivedPackedBoolSlice {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as ArchivedLayout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl ArchivedValidate for ArchivedPackedBoolSlice {
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let _guard = context.guard()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        let len = u32_to_usize(archived.len, || ZebinError::ValidationError {
            message: "Archived packed bool length exceeds usize range".to_string(),
            pos: ptr as usize,
        })?;

        if len > 0 {
            let data_ptr = archived
                .ptr
                .as_ref()
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Null pointer in non-empty packed bool slice".to_string(),
                    pos: ptr as usize,
                })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            let packed_len = packed_byte_len(len, 1).map_err(|_| ZebinError::ValidationError {
                message: "Packed bool byte length overflow".to_string(),
                pos: ptr as usize,
            })?;
            context.check_range(data_ptr, packed_len)?;
        }

        Ok(())
    }
}

impl<'a> ArchivedDecode<'a> for ArchivedPackedBoolSlice {
    type View = &'a Self;

    unsafe fn decode_view<C: ArchivedValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as ArchivedValidate>::validate(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

/// Archived packed small-integer slice. Values are stored using `BITS` bits.
#[repr(C)]
pub struct ArchivedPackedU8Slice<const BITS: u8> {
    ptr: Option<RelPtr<u8>>,
    len: u32,
}

impl<const BITS: u8> ArchivedPackedU8Slice<BITS> {
    pub fn len(&self) -> usize {
        u32_to_usize(self.len, || ZebinError::ValidationError {
            message: "Archived packed integer length exceeds usize range".to_string(),
            pos: self as *const _ as usize,
        })
        .expect("validated packed integer length should fit in usize")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<u8> {
        let len = self.len();
        if index >= len {
            return None;
        }
        let bytes = unsafe { self.packed_bytes() };
        let bit_offset = index * usize::from(BITS);
        Some(read_packed_bits(bytes, bit_offset, usize::from(BITS)))
    }

    unsafe fn packed_bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        let len = self.len();
        let byte_len = packed_byte_len(len, usize::from(BITS)).expect("packed length should fit");
        let ptr = self
            .ptr
            .as_ref()
            .expect("non-empty packed integer slice must have a pointer");
        unsafe { core::slice::from_raw_parts(ptr.as_ptr(), byte_len) }
    }
}

impl<const BITS: u8> ArchivedLayout for ArchivedPackedU8Slice<BITS> {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as ArchivedLayout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl<const BITS: u8> ArchivedValidate for ArchivedPackedU8Slice<BITS> {
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        ptr: *const Self,
        context: &mut C,
    ) -> Result<(), ZebinError> {
        let _guard = context.guard()?;
        context.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        context.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        let len = u32_to_usize(archived.len, || ZebinError::ValidationError {
            message: "Archived packed integer length exceeds usize range".to_string(),
            pos: ptr as usize,
        })?;

        if len > 0 {
            let data_ptr = archived
                .ptr
                .as_ref()
                .ok_or_else(|| ZebinError::ValidationError {
                    message: "Null pointer in non-empty packed integer slice".to_string(),
                    pos: ptr as usize,
                })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            let packed_len = packed_byte_len(len, usize::from(BITS)).map_err(|_| {
                ZebinError::ValidationError {
                    message: "Packed integer byte length overflow".to_string(),
                    pos: ptr as usize,
                }
            })?;
            context.check_range(data_ptr, packed_len)?;

            let max = if BITS == 8 {
                u8::MAX
            } else {
                (1u8 << BITS) - 1
            };
            let bytes = unsafe { core::slice::from_raw_parts(data_ptr, packed_len) };
            for index in 0..len {
                let bit_offset = index * usize::from(BITS);
                let value = read_packed_bits(bytes, bit_offset, usize::from(BITS));
                if value > max {
                    return Err(ZebinError::ValidationError {
                        message: "Packed integer value out of range".to_string(),
                        pos: ptr as usize,
                    });
                }
            }
        }

        Ok(())
    }
}

impl<'a, const BITS: u8> ArchivedDecode<'a> for ArchivedPackedU8Slice<BITS> {
    type View = &'a Self;

    unsafe fn decode_view<C: ArchivedValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as ArchivedValidate>::validate(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

/// Packed sequence state shared by the packed APIs.
#[doc(hidden)]
pub struct PackedSequenceState {
    bytes: Vec<u8>,
    cursor: usize,
    start_pos: Option<usize>,
}

impl PackedSequenceState {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            cursor: 0,
            start_pos: None,
        }
    }
}

impl ArchiveState for PackedSequenceState {
    type Resolver = usize;

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
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

/// Borrowed packed sequence wrapper.
pub struct PackedSlice<'a, T, const BITS: u8> {
    values: &'a [T],
}

/// Owned packed sequence wrapper.
pub struct PackedVec<T, const BITS: u8> {
    values: Vec<T>,
}

/// Backward-compatible alias for bit-packed booleans.
pub type PackedBoolSlice<'a> = PackedSlice<'a, bool, 1>;
/// Backward-compatible alias for packed small integers.
pub type PackedU8Slice<'a, const BITS: u8> = PackedSlice<'a, u8, BITS>;
/// Owned bit-packed booleans.
pub type PackedBoolVec = PackedVec<bool, 1>;
/// Owned packed small integers.
pub type PackedU8Vec<const BITS: u8> = PackedVec<u8, BITS>;

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

impl<T, const BITS: u8> PackedVec<T, BITS> {
    pub fn new(values: Vec<T>) -> Self {
        Self { values }
    }

    pub fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }

    pub fn into_iter(self) -> alloc::vec::IntoIter<T> {
        self.values.into_iter()
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

impl Archive for PackedSlice<'_, bool, 1> {
    type Archived = ArchivedPackedBoolSlice;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.values.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedPackedBoolSlice {
            ptr,
            len: usize_to_u32(self.values.len(), || ZebinError::WriteError)?,
        })
    }
}

impl ArchiveBuilder for PackedSlice<'_, bool, 1> {
    type State<'a>
        = PackedSequenceState
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_bools(self.values)))
    }
}

impl Archive for PackedVec<bool, 1> {
    type Archived = ArchivedPackedBoolSlice;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.values.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedPackedBoolSlice {
            ptr,
            len: usize_to_u32(self.values.len(), || ZebinError::WriteError)?,
        })
    }
}

impl ArchiveBuilder for PackedVec<bool, 1> {
    type State<'a>
        = PackedSequenceState
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_bools(self.values.as_slice())))
    }
}

impl<const BITS: u8> Archive for PackedSlice<'_, u8, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.values.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedPackedU8Slice {
            ptr,
            len: usize_to_u32(self.values.len(), || ZebinError::WriteError)?,
        })
    }
}

impl<const BITS: u8> ArchiveBuilder for PackedSlice<'_, u8, BITS> {
    type State<'a>
        = PackedSequenceState
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_small_values(
            self.values,
            usize::from(BITS),
        )?))
    }
}

impl<const BITS: u8> Archive for PackedVec<u8, BITS> {
    type Archived = ArchivedPackedU8Slice<BITS>;
    type Resolver = usize;

    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        let ptr = if self.values.is_empty() {
            None
        } else {
            Some(RelPtr::new(archive_pos, resolver)?)
        };
        Ok(ArchivedPackedU8Slice {
            ptr,
            len: usize_to_u32(self.values.len(), || ZebinError::WriteError)?,
        })
    }
}

impl<const BITS: u8> ArchiveBuilder for PackedVec<u8, BITS> {
    type State<'a>
        = PackedSequenceState
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(PackedSequenceState::new(pack_small_values(
            self.values.as_slice(),
            usize::from(BITS),
        )?))
    }
}
