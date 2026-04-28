use alloc::{string::ToString, vec::Vec};
use core::{marker::PhantomData, num::NonZeroUsize, ops::Deref, task::Poll};

use crate::{
    ArchiveBuilder, ArchiveState, ArchivedDecode, ArchivedLayout, ArchivedValidate,
    ArchivedValidationContext, ByteSink, LayoutSink, ZebinError, core::schema::ObjectEncoding,
    traits::Archive,
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
    const MAX_BYTES: usize;

    fn to_u64(self) -> u64;
    fn try_from_u64(value: u64) -> Result<Self, ZebinError>;
}

macro_rules! impl_varint_number {
    ($($t:ty),* $(,)?) => {
        $(
            impl VarIntNumber for $t {
                const MAX_BYTES: usize = (core::mem::size_of::<Self>() * 8 + 6) / 7;

                fn to_u64(self) -> u64 {
                    self as u64
                }

                fn try_from_u64(value: u64) -> Result<Self, ZebinError> {
                    <$t>::try_from(value).map_err(|_| ZebinError::ValidationError {
                        message: "VarInt value out of range".to_string(),
                        pos: 0,
                    })
                }
            }
        )*
    };
}

impl_varint_number!(u8, u16, u32, u64, usize);

fn encoded_len_u64(value: u64) -> usize {
    let mut value = value;
    let mut len = 1usize;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn encode_u64(mut value: u64, out: &mut [u8]) {
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
    }
}

fn decode_u64<T: VarIntNumber, C: ArchivedValidationContext + ?Sized>(
    bytes: *const u8,
    context: &mut C,
) -> Result<(T, usize), ZebinError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut consumed = 0usize;
    loop {
        if consumed >= T::MAX_BYTES {
            return Err(ZebinError::ValidationError {
                message: "VarInt exceeds maximum length".to_string(),
                pos: bytes as usize,
            });
        }
        let byte_ptr = unsafe { bytes.add(consumed) };
        context.check_range(byte_ptr, 1)?;
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
            return Err(ZebinError::ValidationError {
                message: "VarInt shift overflow".to_string(),
                pos: bytes as usize,
            });
        }
    }
}

impl<T> ArchivedLayout for VarInt<T>
where
    T: VarIntNumber,
{
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::VarInt;

    fn encoded_len(archived: &Self) -> usize
    where
        Self: Sized,
    {
        encoded_len_u64(archived.value.to_u64())
    }

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        encode_u64(archived.value.to_u64(), out);
    }
}

impl<T> ArchivedValidate for VarInt<T>
where
    T: VarIntNumber,
{
    unsafe fn validate<C: ArchivedValidationContext + ?Sized>(
        _ptr: *const Self,
        _context: &mut C,
    ) -> Result<(), ZebinError> {
        Ok(())
    }
}

impl<'a, T: VarIntNumber + 'a> ArchivedDecode<'a> for VarInt<T>
where
    T: VarIntNumber,
{
    type View = VarIntView<T>;

    unsafe fn decode_view<C: ArchivedValidationContext + ?Sized>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), ZebinError> {
        let (value, consumed) = decode_u64::<T, C>(ptr, context)?;
        Ok((VarIntView { value }, consumed))
    }
}

impl<T> Archive for VarInt<T>
where
    T: VarIntNumber,
{
    type Archived = Self;
    type Resolver = ();

    fn resolve(
        &self,
        _archive_pos: usize,
        _resolver: Self::Resolver,
    ) -> Result<Self::Archived, ZebinError> {
        Ok(*self)
    }
}

pub struct VarIntArchiveState<T: VarIntNumber> {
    bytes: Vec<u8>,
    cursor: usize,
    _marker: PhantomData<T>,
}

impl<T: VarIntNumber> VarIntArchiveState<T> {
    fn new(value: T) -> Self {
        let mut bytes = vec![0u8; encoded_len_u64(value.to_u64())];
        encode_u64(value.to_u64(), &mut bytes);
        Self {
            bytes,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}

impl<T> ArchiveBuilder for VarInt<T>
where
    T: VarIntNumber,
{
    type State<'a>
        = VarIntArchiveState<T>
    where
        Self: 'a;

    fn begin(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntArchiveState::new(self.value))
    }
}

impl<T: VarIntNumber> ArchiveState for VarIntArchiveState<T> {
    type Resolver = ();

    fn poll<E: ByteSink + LayoutSink + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<Poll<Self::Resolver>, ZebinError> {
        let written = encoder.write(&self.bytes[self.cursor..])?;
        self.cursor += written;
        if self.cursor < self.bytes.len() {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}
