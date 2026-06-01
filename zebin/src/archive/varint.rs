use core::{marker::PhantomData, ops::Deref, task::Poll};

#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::prelude::*;

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

/// Borrowed view over a deserialized varint.
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

impl<T: Default> ArchivedDefault for VarIntView<T> {
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

            impl Deserialize<$t> for VarInt<$t> {
                fn deserialize(&self) -> Result<$t, ZebinError> {
                    Ok(self.value)
                }
            }

            impl Deserialize<VarInt<$t>> for VarInt<$t> {
                fn deserialize(&self) -> Result<VarInt<$t>, ZebinError> {
                    Ok(*self)
                }
            }

            impl Deserialize<$t> for VarIntView<$t> {
                fn deserialize(&self) -> Result<$t, ZebinError> {
                    Ok(self.value)
                }
            }

            impl Deserialize<VarInt<$t>> for VarIntView<$t> {
                fn deserialize(&self) -> Result<VarInt<$t>, ZebinError> {
                    Ok(VarInt { value: self.value })
                }
            }
        )*
    };
}

impl_varint_number!(u8 => 2, u16 => 3, u32 => 5, u64 => 10, usize => 10);

/// Zero-sized deserialize marker for a varint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchivedVarInt<T> {
    _marker: PhantomData<T>,
}

impl<T> ArchivedLayout for ArchivedVarInt<T> {
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::VarInt;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::VarInt;
}

impl<T> Archive for ArchivedVarInt<T> {
    type Archived = Self;
}

pub(crate) fn serialized_len_u64(value: u64) -> usize {
    let mut value = value;
    let mut len = 1usize;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

pub(crate) fn serialize_u64(mut value: u64, out: &mut [u8]) {
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

pub(crate) fn deserialize_u64<'a, T, C, Cr>(
    cursor: &mut Cr,
    context: &mut C,
) -> Result<T, AccessError>
where
    T: VarIntNumber,
    C: ValidationContext + ?Sized,
    Cr: Cursor<'a> + ?Sized,
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

impl<T> Access for ArchivedVarInt<T>
where
    T: VarIntNumber,
{
    type View<'a>
        = VarIntView<T>
    where
        Self: 'a;
    #[cfg(feature = "alloc")]
    type AccessStrategy = ForwardSequenceStrategy;

    fn access<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
        Self: 'a,
    {
        let value = deserialize_u64::<T, C, Cr>(cursor, context)?;
        Ok(VarIntView { value })
    }

    fn validate<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        deserialize_u64::<T, C, Cr>(cursor, context).map(|_| ())
    }
}

impl<T> Archive for VarInt<T>
where
    T: VarIntNumber,
{
    type Archived = ArchivedVarInt<T>;
}

pub trait ToVarIntNumber<T: VarIntNumber> {
    fn to_varint_number(&self) -> T;
}

impl<T: VarIntNumber> ToVarIntNumber<T> for VarInt<T> {
    fn to_varint_number(&self) -> T {
        self.get()
    }
}

impl<T: VarIntNumber> ToVarIntNumber<T> for VarIntView<T> {
    fn to_varint_number(&self) -> T {
        self.get()
    }
}

pub struct VarIntSerializer<T: VarIntNumber, I = ()> {
    bytes: [u8; 10],
    len: u8,
    cursor: u8,
    _phantom: PhantomData<(T, I)>,
}

impl<T: VarIntNumber, I> VarIntSerializer<T, I> {
    pub(crate) fn new_empty() -> Self {
        Self {
            bytes: [0u8; 10],
            len: 0,
            cursor: 0,
            _phantom: PhantomData,
        }
    }
}

impl<T: VarIntNumber, I> Serializer for VarIntSerializer<T, I>
where
    I: ToVarIntNumber<T>,
{
    type Input = I;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut dyn CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        let val = item.to_varint_number().to_u64();
        let len = serialized_len_u64(val);
        self.bytes.fill(0);
        serialize_u64(val, &mut self.bytes[..len]);
        self.len = len as u8;
        self.cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending(&mut self, sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        let mut cursor = self.cursor as usize;
        let len = self.len as usize;
        let remaining = len - cursor;
        let progress = sink
            .write(&self.bytes[cursor..len])?
            .advance_cursor(&mut cursor, remaining);
        self.cursor = cursor as u8;
        Ok(progress)
    }

    fn finish(self, _sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<T> Serialize for VarInt<T>
where
    T: VarIntNumber,
{
    type Input<'a>
        = VarInt<T>
    where
        Self: 'a;
    type Serializer<'a>
        = VarIntSerializer<T, VarInt<T>>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        VarIntSerializer::new_empty()
    }
}

impl<T> Serialize for VarIntView<T>
where
    T: VarIntNumber,
{
    type Input<'a>
        = VarIntView<T>
    where
        Self: 'a;
    type Serializer<'a>
        = VarIntSerializer<T, VarIntView<T>>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        VarIntSerializer::new_empty()
    }
}

impl<T: VarIntNumber> MeasureBody for VarInt<T> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        Ok(serialized_len_u64(self.get().to_u64()))
    }
}

impl<T: VarIntNumber> MeasureBody for VarIntView<T> {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        Ok(serialized_len_u64(self.get().to_u64()))
    }
}

impl<'a, T: 'a> ArchivedField<'a> for VarIntView<T> {}
