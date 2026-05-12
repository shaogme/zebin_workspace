use core::{marker::PhantomData, ops::Deref, task::Poll};

use crate::{
    core::schema::{FieldEncoding, ObjectEncoding},
    error::{AccessError, ZebinError},
    read::Cursor,
    traits::{
        Archive, ArchivedDefault, ByteSink, Decode, Restore, SchemaAware, Serialize, SerializeState,
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

impl<T: VarIntNumber> Archive for VarIntView<T> {
    type Archived = ArchivedVarInt<T>;
}

impl<T> SchemaAware for VarIntView<T> {
    fn stable_schema_key(&self) -> u32 {
        0
    }

    fn schema_revision(&self) -> u32 {
        0
    }
}

impl<T: Default + 'static> ArchivedDefault for VarIntView<T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: VarIntView<()> = VarIntView { value: () };
        unsafe { &*(&DEFAULT as *const VarIntView<()> as *const VarIntView<T>) }
    }
}

pub trait VarIntNumber: Copy {
    const MAX_BYTES: usize;

    fn to_u64(self) -> u64;
    fn try_from_u64(value: u64) -> Result<Self, AccessError>;
}

macro_rules! impl_varint_number {
    ($($t:ty => $max_bytes:expr),* $(,)?) => {
        $(
            impl VarIntNumber for $t {
                const MAX_BYTES: usize = $max_bytes;

                fn to_u64(self) -> u64 {
                    self as u64
                }

                fn try_from_u64(value: u64) -> Result<Self, AccessError> {
                    <$t>::try_from(value).map_err(|_| AccessError::ValidationError {
                        message: "VarInt value out of range",
                        pos: 0,
                    })
                }
            }

            impl Restore<$t> for VarInt<$t> {
                fn restore(&self) -> Result<$t, ZebinError> {
                    Ok(self.value)
                }
            }

            impl Restore<VarInt<$t>> for VarInt<$t> {
                fn restore(&self) -> Result<VarInt<$t>, ZebinError> {
                    Ok(*self)
                }
            }

            impl Restore<$t> for VarIntView<$t> {
                fn restore(&self) -> Result<$t, ZebinError> {
                    Ok(self.value)
                }
            }

            impl Restore<VarInt<$t>> for VarIntView<$t> {
                fn restore(&self) -> Result<VarInt<$t>, ZebinError> {
                    Ok(VarInt { value: self.value })
                }
            }
        )*
    };
}

impl_varint_number!(u8 => 2, u16 => 3, u32 => 5, u64 => 10, usize => 10);

/// Zero-sized decode marker for a varint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchivedVarInt<T> {
    _marker: PhantomData<T>,
}

impl<T> Archive for ArchivedVarInt<T> {
    type Archived = Self;
}

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
        if value == 0 || cursor >= out.len() {
            break;
        }
    }
}

pub(crate) fn decode_u64<T, C>(cursor: &mut Cursor<'_>, context: &mut C) -> Result<T, AccessError>
where
    T: VarIntNumber,
    C: ValidationContext + ?Sized,
{
    let start_pos = cursor.pos();
    let mut value = 0u64;
    let mut shift = 0u32;
    let mut consumed = 0usize;
    loop {
        if consumed >= T::MAX_BYTES {
            return Err(context.validation_error("VarInt exceeds maximum length", start_pos));
        }
        let byte = cursor.read_u8(context)?;
        let payload = u64::from(byte & 0x7F);
        value |= payload << shift;
        consumed += 1;
        if byte & 0x80 == 0 {
            return T::try_from_u64(value);
        }
        shift += 7;
        if shift >= 64 {
            return Err(context.validation_error("VarInt shift overflow", start_pos));
        }
    }
}

impl<'a, T> Decode<'a> for ArchivedVarInt<T>
where
    T: VarIntNumber + 'a,
{
    type View = VarIntView<T>;

    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::VarInt;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::VarInt;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        let value = decode_u64::<T, C>(cursor, context)?;
        Ok(VarIntView { value })
    }
}

impl<T> Archive for VarInt<T>
where
    T: VarIntNumber,
{
    type Archived = ArchivedVarInt<T>;
}

pub struct VarIntArchiveState<T: VarIntNumber> {
    bytes: [u8; 10],
    len: u8,
    cursor: u8,
    _marker: PhantomData<T>,
}

impl<T: VarIntNumber> VarIntArchiveState<T> {
    pub(crate) fn new(value: T) -> Self {
        let val = value.to_u64();
        let len = encoded_len_u64(val);
        let mut bytes = [0u8; 10];
        encode_u64(val, &mut bytes[..len]);
        Self {
            bytes,
            len: len as u8,
            cursor: 0,
            _marker: PhantomData,
        }
    }
}

impl<'a, T: VarIntNumber> SerializeState<'a> for VarIntArchiveState<T> {
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        let written = encoder.write(&self.bytes[self.cursor as usize..self.len as usize])?;
        self.cursor += written as u8;
        if self.cursor < self.len {
            Ok(Poll::Pending)
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

impl<T> Serialize for VarInt<T>
where
    T: VarIntNumber,
{
    type State<'a>
        = VarIntArchiveState<T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntArchiveState::new(self.get()))
    }
}

impl<T> Serialize for VarIntView<T>
where
    T: VarIntNumber,
{
    type State<'a>
        = VarIntArchiveState<T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        Ok(VarIntArchiveState::new(self.get()))
    }
}
