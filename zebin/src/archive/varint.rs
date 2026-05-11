use core::{num::NonZeroUsize, ops::Deref};

use crate::{
    core::schema::ObjectEncoding,
    error::{AccessError, ArchiveError, ValidateError, ZebinError},
    read::ResolvedLayout,
    traits::{
        Access, Archive, ArchiveHeader, ArchivedDefault, Layout, Restore, RestoreFromView, Validate,
    },
    validation::context::ValidationContext,
};

/// Unsigned integers that are serialized with a variable-length encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarInt<T> {
    value: T,
}

impl<T> VarInt<T> {
    pub fn new(value: T) -> Self {
        Self { value }
    }

    pub fn get(&self) -> T
    where
        T: Copy,
    {
        self.value
    }

    pub fn into_inner(self) -> T {
        self.value
    }
}

/// Borrowed view over a decoded varint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarIntView<T> {
    value: T,
}

impl<T> VarIntView<T> {
    pub fn get(&self) -> T
    where
        T: Copy,
    {
        self.value
    }
}

impl<T> Deref for VarIntView<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

pub trait VarIntNumber: Copy {
    type Archived: Layout + Validate + Copy + Send + Sync + 'static;
    const MAX_BYTES: usize;

    fn to_u64(self) -> u64;
    fn try_from_u64(value: u64) -> Result<Self, ValidateError>;
    fn from_archived(archived: Self::Archived) -> Self;
    fn to_archived(self) -> Self::Archived;
}

macro_rules! impl_varint_number {
    ($t:ty, $archived:ident, $max_bytes:expr) => {
        #[repr(C)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $archived {
            bytes: [u8; $max_bytes],
        }

        impl $archived {
            pub fn get(self) -> $t {
                let mut value = 0u64;
                let mut shift = 0u32;
                for &byte in &self.bytes {
                    let payload = u64::from(byte & 0x7F);
                    value |= payload << shift;
                    if byte & 0x80 == 0 {
                        break;
                    }
                    shift += 7;
                }
                value as $t
            }
        }

        impl ArchivedDefault for $archived {
            fn archived_default() -> &'static Self {
                static DEFAULT: $archived = $archived {
                    bytes: [0u8; $max_bytes],
                };
                &DEFAULT
            }
        }

        impl Layout for $archived {
            const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
            const ENCODING: ObjectEncoding = ObjectEncoding::VarInt;

            fn size_hint(&self) -> usize {
                encoded_len_u64(self.get() as u64)
            }

            fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
                encode_u64(archived.get() as u64, out);
            }
        }

        impl Validate for $archived {
            unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
            where
                H: ArchiveHeader,
                C: ValidationContext<H> + ?Sized,
            {
                let mut guard = context.guard()?;
                guard.check_range(ptr as *const u8, $max_bytes)?;
                Ok(())
            }
        }

        impl<'a> Access<'a> for $archived {
            type View = &'a Self;

            unsafe fn access<H, C>(
                ptr: *const u8,
                context: &mut C,
            ) -> Result<(Self::View, usize), AccessError>
            where
                H: ArchiveHeader,
                C: ValidationContext<H> + ?Sized,
            {
                let typed_ptr = ptr as *const Self;
                unsafe {
                    <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
                }
                Ok((unsafe { &*typed_ptr }, $max_bytes))
            }
        }

        impl Restore<$t> for $archived {
            fn restore(&self) -> Result<$t, crate::error::ZebinError> {
                Ok(self.get())
            }
        }

        impl<'a, H: ArchiveHeader> RestoreFromView<'a, $t, H> for $archived {
            fn restore_from_view(
                &self,
                _layout: &ResolvedLayout<'a, H>,
            ) -> Result<$t, crate::error::ZebinError> {
                Ok(self.get())
            }
        }

        impl Restore<crate::archive::varint::VarInt<$t>> for $archived {
            fn restore(
                &self,
            ) -> Result<crate::archive::varint::VarInt<$t>, crate::error::ZebinError> {
                Ok(crate::archive::varint::VarInt { value: self.get() })
            }
        }

        impl<'a, H: ArchiveHeader> RestoreFromView<'a, crate::archive::varint::VarInt<$t>, H>
            for $archived
        {
            fn restore_from_view(
                &self,
                _layout: &ResolvedLayout<'a, H>,
            ) -> Result<crate::archive::varint::VarInt<$t>, crate::error::ZebinError> {
                self.restore()
            }
        }

        impl VarIntNumber for $t {
            type Archived = $archived;
            const MAX_BYTES: usize = $max_bytes;

            fn to_u64(self) -> u64 {
                self as u64
            }

            fn try_from_u64(value: u64) -> Result<Self, ValidateError> {
                <$t>::try_from(value).map_err(|_| ValidateError::ValidationError {
                    message: "VarInt value out of range",
                    pos: 0,
                })
            }

            fn from_archived(archived: Self::Archived) -> Self {
                archived.get()
            }

            fn to_archived(self) -> Self::Archived {
                let mut bytes = [0u8; $max_bytes];
                encode_u64(self as u64, &mut bytes);
                $archived { bytes }
            }
        }

        impl Restore<$t> for VarInt<$t> {
            fn restore(&self) -> Result<$t, ZebinError> {
                Ok(self.value)
            }
        }

        impl<'a, H: ArchiveHeader> RestoreFromView<'a, $t, H> for VarInt<$t> {
            fn restore_from_view(&self, _layout: &ResolvedLayout<'a, H>) -> Result<$t, ZebinError> {
                Ok(self.value)
            }
        }

        impl Restore<VarInt<$t>> for VarInt<$t> {
            fn restore(&self) -> Result<VarInt<$t>, ZebinError> {
                Ok(*self)
            }
        }

        impl<'a, H: ArchiveHeader> RestoreFromView<'a, VarInt<$t>, H> for VarInt<$t> {
            fn restore_from_view(
                &self,
                _layout: &ResolvedLayout<'a, H>,
            ) -> Result<VarInt<$t>, ZebinError> {
                Ok(*self)
            }
        }

        impl Restore<$t> for VarIntView<$t> {
            fn restore(&self) -> Result<$t, ZebinError> {
                Ok(self.value)
            }
        }

        impl<'a, H: ArchiveHeader> RestoreFromView<'a, $t, H> for VarIntView<$t> {
            fn restore_from_view(&self, _layout: &ResolvedLayout<'a, H>) -> Result<$t, ZebinError> {
                Ok(self.value)
            }
        }

        impl Restore<VarInt<$t>> for VarIntView<$t> {
            fn restore(&self) -> Result<VarInt<$t>, ZebinError> {
                Ok(VarInt { value: self.value })
            }
        }

        impl<'a, H: ArchiveHeader> RestoreFromView<'a, VarInt<$t>, H> for VarIntView<$t> {
            fn restore_from_view(
                &self,
                _layout: &ResolvedLayout<'a, H>,
            ) -> Result<VarInt<$t>, ZebinError> {
                Ok(VarInt { value: self.value })
            }
        }
    };
}

impl_varint_number!(u8, ArchivedVarIntU8, 2);
impl_varint_number!(u16, ArchivedVarIntU16, 3);
impl_varint_number!(u32, ArchivedVarIntU32, 5);
impl_varint_number!(u64, ArchivedVarIntU64, 10);
impl_varint_number!(usize, ArchivedVarIntUsize, 10);

pub(crate) fn encoded_len_u64(value: u64) -> usize {
    let mut value = value;
    let mut len = 1usize;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

pub(crate) fn encode_u64(mut value: u64, out: &mut [u8]) {
    let mut cursor = 0usize;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out[cursor] = byte;
        cursor += 1;
        if value == 0 {
            break;
        }
        if cursor >= out.len() {
            break;
        }
    }
}

fn decode_u64<T, H, C>(bytes: *const u8, context: &mut C) -> Result<(T, usize), ValidateError>
where
    T: VarIntNumber,
    H: ArchiveHeader,
    C: ValidationContext<H> + ?Sized,
{
    let mut guard = context.guard()?;
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut consumed = 0usize;
    loop {
        if consumed >= T::MAX_BYTES {
            return Err(guard.validation_error("VarInt exceeds maximum length", bytes as usize));
        }
        let byte_ptr = unsafe { bytes.add(consumed) };
        guard.check_range(byte_ptr, 1)?;
        let byte = unsafe { *byte_ptr };
        let payload = u64::from(byte & 0x7F);
        value |= payload << shift;
        consumed += 1;
        if byte & 0x80 == 0 {
            let value = T::try_from_u64(value)?;
            return Ok((value, consumed));
        }
        shift += 7;
        if shift >= 64 {
            return Err(guard.validation_error("VarInt shift overflow", bytes as usize));
        }
    }
}

impl<T> Layout for VarInt<T>
where
    T: VarIntNumber,
{
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const ENCODING: ObjectEncoding = ObjectEncoding::VarInt;

    fn size_hint(&self) -> usize {
        encoded_len_u64(self.value.to_u64())
    }

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        encode_u64(archived.value.to_u64(), out);
    }
}

impl<T> Validate for VarInt<T>
where
    T: VarIntNumber,
{
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let (value, _) = decode_u64::<T, H, C>(ptr as *const u8, context)?;
        let _ = value;
        Ok(())
    }
}

impl<'a, T: VarIntNumber + 'a> Access<'a> for VarInt<T>
where
    T: VarIntNumber,
{
    type View = VarIntView<T>;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let (value, consumed) = decode_u64::<T, H, C>(ptr, context)?;
        Ok((VarIntView { value }, consumed))
    }
}

impl<T> Archive for VarInt<T>
where
    T: VarIntNumber,
{
    type Archived = T::Archived;
    type Resolver = ();

    fn resolve(
        &self,
        _archive_pos: usize,
        _resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError> {
        Ok(self.value.to_archived())
    }
}
