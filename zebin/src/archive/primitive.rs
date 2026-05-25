use crate::{
    ZebinError,
    error::DecodeError,
    read::Cursor,
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Encode, Encoder, FixedLayout,
        Restore, SchemaAware,
    },
    validation::context::ValidationContext,
};
use core::{num::NonZeroUsize, task::Poll};

/// Byte-oriented encoder used by fixed-width primitive encoders.
pub struct ByteEncoder<'a, const N: usize, T = ()> {
    bytes: [u8; N],
    cursor: usize,
    _phantom: core::marker::PhantomData<&'a T>,
}

impl<'a, const N: usize, T> ByteEncoder<'a, N, T> {
    pub fn new(bytes: [u8; N]) -> Self {
        Self {
            bytes,
            cursor: 0,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, const N: usize, T> Encoder<'a> for ByteEncoder<'a, N, T> {
    type Input = &'a T;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        let remaining = N - self.cursor;
        Ok(sink
            .write(&self.bytes[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

macro_rules! impl_archive_for_primitive {
    ($($t:ty),* $(,)?) => {
        $(
            impl FixedLayout for $t {
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                };

                fn write_fixed(archived: &Self, out: &mut [u8]) {
                    crate::utils::byteops::copy_exact(out, &archived.to_le_bytes());
                }
            }

            impl ArchivedLayout for $t {
                const FIXED_SIZE: Option<usize> = Some(core::mem::size_of::<Self>());
                const ALIGNMENT: NonZeroUsize = unsafe {
                    NonZeroUsize::new_unchecked(core::mem::size_of::<Self>())
                };
            }

            impl<'a> Decode<'a> for $t {
                type View = Self;

                fn decode<C>(
                    cursor: &mut Cursor<'a>,
                    context: &mut C,
                ) -> Result<Self::View, DecodeError>
                where
                    C: ValidationContext + ?Sized,
                {
                    let bytes = cursor.read_exact(core::mem::size_of::<Self>(), context)?;
                    let mut fixed = [0u8; core::mem::size_of::<Self>()];
                    fixed.copy_from_slice(bytes);
                    Ok(<$t>::from_le_bytes(fixed))
                }

                fn validate<C>(
                    cursor: &mut Cursor<'a>,
                    context: &mut C,
                ) -> Result<(), DecodeError>
                where
                    C: ValidationContext + ?Sized,
                {
                    cursor.advance(core::mem::size_of::<Self>(), context)
                }
            }

            impl Archive for $t {
                type Archived = $t;
            }

            impl Encode for $t {
                type Encoder<'a> = ByteEncoder<'a, { core::mem::size_of::<$t>() }, $t> where Self: 'a;

                fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
                    Ok(ByteEncoder::new(self.to_le_bytes()))
                }
            }

            impl ArchivedDefault for $t {
                fn archived_default() -> &'static Self {
                    static DEFAULT: $t = 0 as $t;
                    &DEFAULT
                }
            }

            impl Restore<$t> for $t {
                fn restore(&self) -> Result<$t, ZebinError> {
                    Ok(*self)
                }
            }

            impl SchemaAware for $t {
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
        )*
    };
}

impl_archive_for_primitive!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl FixedLayout for bool {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const SIZE: usize = 1;

    fn write_fixed(archived: &Self, out: &mut [u8]) {
        out[0] = *archived as u8;
    }
}

impl ArchivedLayout for bool {
    const FIXED_SIZE: Option<usize> = Some(1);
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
}

impl<'a> Decode<'a> for bool {
    type View = bool;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        let value = cursor.read_u8(context)?;
        match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(context.validation_error("Invalid bool value", pos)),
        }
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Self::decode(cursor, context).map(|_| ())
    }
}

impl Archive for bool {
    type Archived = bool;
}

impl Encode for bool {
    type Encoder<'a>
        = ByteEncoder<'a, 1, bool>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(ByteEncoder::new([*self as u8]))
    }
}

impl ArchivedDefault for bool {
    fn archived_default() -> &'static Self {
        &false
    }
}

impl Restore<bool> for bool {
    fn restore(&self) -> Result<bool, ZebinError> {
        Ok(*self)
    }
}

impl SchemaAware for bool {
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

pub struct UnitEncoder<'a, T = ()> {
    _phantom: core::marker::PhantomData<&'a T>,
}

impl<'a, T> Encoder<'a> for UnitEncoder<'a, T> {
    type Input = &'a T;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        _sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }

    fn poll_pending<S: ByteSink + ?Sized>(
        &mut self,
        _sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl FixedLayout for () {
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const SIZE: usize = 0;

    fn write_fixed(_archived: &Self, _out: &mut [u8]) {}
}

impl ArchivedLayout for () {
    const FIXED_SIZE: Option<usize> = Some(0);
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
}

impl<'a> Decode<'a> for () {
    type View = ();

    fn decode<C>(_cursor: &mut Cursor<'a>, _context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(())
    }

    fn validate<C>(_cursor: &mut Cursor<'a>, _context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(())
    }
}

impl Archive for () {
    type Archived = ();
}

impl Encode for () {
    type Encoder<'a> = UnitEncoder<'a, ()>;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        Ok(UnitEncoder {
            _phantom: core::marker::PhantomData,
        })
    }
}

impl ArchivedDefault for () {
    fn archived_default() -> &'static Self {
        &()
    }
}

impl Restore<()> for () {
    fn restore(&self) -> Result<(), ZebinError> {
        Ok(())
    }
}

impl SchemaAware for () {
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
