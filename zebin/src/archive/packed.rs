use core::num::NonZeroUsize;

use crate::{
    core::rel_ptr::RelPtr,
    error::{AccessError, ArchiveError, ValidateError},
    traits::{Access, Archive, ArchivedDefault, Layout, Validate},
    utils::{
        byteops,
        num::{u32_to_usize, usize_to_u32},
    },
    validation::context::ValidationContext,
};

pub(crate) fn packed_byte_len(
    value_count: usize,
    bits_per_value: usize,
) -> Result<usize, ValidateError> {
    let total_bits =
        value_count
            .checked_mul(bits_per_value)
            .ok_or(ValidateError::ValidationError {
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

/// Archived packed boolean slice. Values are stored as one bit per item.
#[repr(C)]
pub struct ArchivedPackedBoolSlice {
    pub(crate) ptr: Option<RelPtr<u8>>,
    pub(crate) len: u32,
}

impl ArchivedPackedBoolSlice {
    pub fn len(&self) -> usize {
        u32_to_usize(self.len, || ValidateError::ValidationError {
            message: "Archived packed bool length exceeds usize range",
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

impl ArchivedDefault for ArchivedPackedBoolSlice {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedPackedBoolSlice = ArchivedPackedBoolSlice { ptr: None, len: 0 };
        &DEFAULT
    }
}

impl Layout for ArchivedPackedBoolSlice {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as Layout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl Validate for ArchivedPackedBoolSlice {
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        let len = u32_to_usize(archived.len, || {
            guard.validation_error(
                "Archived packed bool length exceeds usize range",
                ptr as usize,
            )
        })?;

        if len > 0 {
            let data_ptr = archived.ptr.as_ref().ok_or_else(|| {
                guard.validation_error("Null pointer in non-empty packed bool slice", ptr as usize)
            })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            let packed_len = packed_byte_len(len, 1).map_err(|_| {
                guard.validation_error("Packed bool byte length overflow", ptr as usize)
            })?;
            guard.check_range(data_ptr, packed_len)?;
        }

        Ok(())
    }
}

impl<'a> Access<'a> for ArchivedPackedBoolSlice {
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

/// Archived packed small-integer slice. Values are stored using `BITS` bits.
#[repr(C)]
pub struct ArchivedPackedU8Slice<const BITS: u8> {
    pub(crate) ptr: Option<RelPtr<u8>>,
    pub(crate) len: u32,
}

impl<const BITS: u8> ArchivedPackedU8Slice<BITS> {
    pub fn len(&self) -> usize {
        u32_to_usize(self.len, || ValidateError::ValidationError {
            message: "Archived packed integer length exceeds usize range",
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
impl<const BITS: u8> ArchivedDefault for ArchivedPackedU8Slice<BITS> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedPackedU8Slice<8> = ArchivedPackedU8Slice { ptr: None, len: 0 };
        unsafe {
            &*(&DEFAULT as *const ArchivedPackedU8Slice<8> as *const ArchivedPackedU8Slice<BITS>)
        }
    }
}

impl<const BITS: u8> Layout for ArchivedPackedU8Slice<BITS> {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(8).unwrap();

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        byteops::fill(out, 0);
        if let Some(ptr) = &archived.ptr {
            out[0..8].copy_from_slice(&ptr.offset().to_le_bytes());
        }
        <u32 as Layout>::write_archived_bytes(&archived.len, &mut out[8..12]);
    }
}

impl<const BITS: u8> Validate for ArchivedPackedU8Slice<BITS> {
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let archived = unsafe { &*ptr };
        let len = u32_to_usize(archived.len, || {
            guard.validation_error(
                "Archived packed integer length exceeds usize range",
                ptr as usize,
            )
        })?;

        if len > 0 {
            let data_ptr = archived.ptr.as_ref().ok_or_else(|| {
                guard.validation_error(
                    "Null pointer in non-empty packed integer slice",
                    ptr as usize,
                )
            })?;
            let data_ptr = unsafe { data_ptr.as_ptr() };
            let packed_len = packed_byte_len(len, usize::from(BITS)).map_err(|_| {
                guard.validation_error("Packed integer byte length overflow", ptr as usize)
            })?;
            guard.check_range(data_ptr, packed_len)?;

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
                    return Err(
                        guard.validation_error("Packed integer value out of range", ptr as usize)
                    );
                }
            }
        }

        Ok(())
    }
}

impl<'a, const BITS: u8> Access<'a> for ArchivedPackedU8Slice<BITS> {
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: crate::traits::ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

/// Borrowed packed sequence wrapper.
pub struct PackedSlice<'a, T, const BITS: u8> {
    pub(crate) values: &'a [T],
}

/// Backward-compatible alias for bit-packed booleans.
pub type PackedBoolSlice<'a> = PackedSlice<'a, bool, 1>;
/// Backward-compatible alias for packed small integers.
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

impl<const BITS: u8> Archive for PackedSlice<'_, u8, BITS> {
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
