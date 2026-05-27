#[cfg(feature = "alloc")]
use crate::io::FixedSequenceStrategy;
use crate::{prelude::*, utils::byteops};
use core::{num::NonZeroUsize, task::Poll};

pub trait ToBytes<const N: usize> {
    fn to_bytes(&self) -> [u8; N];
}

impl ToBytes<1> for bool {
    fn to_bytes(&self) -> [u8; 1] {
        [*self as u8]
    }
}

macro_rules! impl_to_bytes {
    ($($t:ty),* $(,)?) => {
        $(
            impl ToBytes<{ core::mem::size_of::<$t>() }> for $t {
                fn to_bytes(&self) -> [u8; { core::mem::size_of::<$t>() }] {
                    self.to_le_bytes()
                }
            }
        )*
    };
}

impl_to_bytes!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

/// Byte-oriented serializer used by fixed-width primitive serializers.
///
/// Owns a small fixed-size buffer; `input(value)` immediately serializes the
/// value into the buffer and drops the original ownership.
pub struct ByteSerializer<const N: usize, T = ()> {
    bytes: [u8; N],
    cursor: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<const N: usize, T> ByteSerializer<N, T> {
    pub fn new() -> Self {
        Self {
            bytes: [0; N],
            cursor: 0,
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<const N: usize, T> Default for ByteSerializer<N, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize, T> Serializer for ByteSerializer<N, T>
where
    T: ToBytes<N>,
{
    type Input = T;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.bytes = item.to_bytes();
        self.cursor = 0;
        self.poll_pending(sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let remaining = N - self.cursor;
        Ok(sink
            .write(&self.bytes[self.cursor..])?
            .advance_cursor(&mut self.cursor, remaining))
    }

    fn finish<S: StorageMut + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
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

            impl Access for $t {
                type View<'a>
                    = Self
                where
                    Self: 'a;
                #[cfg(feature = "alloc")]
                type AccessStrategy = crate::io::FixedSequenceStrategy;

                fn access<'a, C>(
                    cursor: &mut Cursor<'a>,
                    context: &mut C,
                ) -> Result<Self::View<'a>, AccessError>
                where
                    C: ValidationContext + ?Sized,
                    Self: 'a
                {
                    let bytes = cursor.read_exact(core::mem::size_of::<Self>(), context)?;
                    let mut fixed = [0u8; core::mem::size_of::<Self>()];
                    byteops::copy_exact(&mut fixed, bytes);
                    Ok(<$t>::from_le_bytes(fixed))
                }

                fn validate<'a, C>(
                    cursor: &mut Cursor<'a>,
                    context: &mut C,
                ) -> Result<(), AccessError>
                where
                    C: ValidationContext + ?Sized,
                {
                    cursor.advance(core::mem::size_of::<Self>(), context)
                }
            }

            impl Archive for $t {
                type Archived = $t;
            }

            impl Serialize for $t {
                type Input<'a> = $t where Self: 'a;
                type Serializer<'a> = ByteSerializer<{ core::mem::size_of::<$t>() }, $t> where Self: 'a;

                fn serializer<'a>() -> Self::Serializer<'a>
                where
                    Self: 'a,
                {
                    ByteSerializer::new()
                }
            }

            impl MeasureBody for $t {
                fn measure_body(&self) -> Result<usize, ZebinError> {
                    Ok(core::mem::size_of::<Self>())
                }
            }

            impl ArchivedDefault for $t {
                fn archived_default() -> &'static Self {
                    static DEFAULT: $t = 0 as $t;
                    &DEFAULT
                }
            }

            impl Deserialize<$t> for $t {
                fn deserialize(&self) -> Result<$t, ZebinError> {
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

impl Access for bool {
    type View<'a>
        = bool
    where
        Self: 'a;
    #[cfg(feature = "alloc")]
    type AccessStrategy = FixedSequenceStrategy;

    fn access<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a,
    {
        let pos = cursor.pos();
        let value = cursor.read_u8(context)?;
        match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(context.validation_error("Invalid bool value", pos)),
        }
    }

    fn validate<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Self::access(cursor, context).map(|_| ())
    }
}

impl Archive for bool {
    type Archived = bool;
}

impl Serialize for bool {
    type Input<'a>
        = bool
    where
        Self: 'a;
    type Serializer<'a>
        = ByteSerializer<1, bool>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        ByteSerializer::new()
    }
}

impl MeasureBody for bool {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        Ok(1)
    }
}

impl ArchivedDefault for bool {
    fn archived_default() -> &'static Self {
        &false
    }
}

impl Deserialize<bool> for bool {
    fn deserialize(&self) -> Result<bool, ZebinError> {
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

pub struct UnitSerializer<T = ()> {
    _phantom: core::marker::PhantomData<T>,
}

impl<T> UnitSerializer<T> {
    pub fn new() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<T> Default for UnitSerializer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Serializer for UnitSerializer<T> {
    type Input = T;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        _item: Self::Input,
        _sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        _sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }

    fn finish<S: StorageMut + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
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

impl Access for () {
    type View<'a>
        = ()
    where
        Self: 'a;
    #[cfg(feature = "alloc")]
    type AccessStrategy = FixedSequenceStrategy;

    fn access<'a, C>(
        _cursor: &mut Cursor<'a>,
        _context: &mut C,
    ) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a,
    {
        Ok(())
    }

    fn validate<'a, C>(_cursor: &mut Cursor<'a>, _context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        Ok(())
    }
}

impl Archive for () {
    type Archived = ();
}

impl Serialize for () {
    type Input<'a>
        = ()
    where
        Self: 'a;
    type Serializer<'a> = UnitSerializer<()>;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        UnitSerializer::new()
    }
}

impl MeasureBody for () {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        Ok(0)
    }
}

impl ArchivedDefault for () {
    fn archived_default() -> &'static Self {
        &()
    }
}

impl Deserialize<()> for () {
    fn deserialize(&self) -> Result<(), ZebinError> {
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

/// Resumable serialization state for `(K, V)`.
///
/// On `input((k, v))` the key `k` is moved into `key_serializer`; the value `v`
/// is parked in `pending_value` until stage 1 begins, at which point it is
/// taken and moved into `value_serializer`.
pub struct Tuple2Serializer<'a, K: Serialize + Archive + 'a, V: Serialize + Archive + 'a> {
    pending_value: Option<V>,
    key_serializer: <K as Serialize>::Serializer<'a>,
    value_serializer: <V as Serialize>::Serializer<'a>,
    key_started: bool,
    value_started: bool,
    stage: u8,
}

impl<'a, K: Serialize + Archive + 'a, V: Serialize + Archive + 'a> Tuple2Serializer<'a, K, V> {
    pub fn new() -> Self {
        Self {
            pending_value: None,
            key_serializer: K::serializer(),
            value_serializer: V::serializer(),
            key_started: false,
            value_started: false,
            stage: 0,
        }
    }
}

impl<'a, K: Serialize + Archive + 'a, V: Serialize + Archive + 'a> Default
    for Tuple2Serializer<'a, K, V>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, K, V> Serializer for Tuple2Serializer<'a, K, V>
where
    K: Serialize<Input<'a> = K> + Archive + 'a,
    V: Serialize<Input<'a> = V> + Archive + 'a,
{
    type Input = (K, V);

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        let (key, value) = item;
        self.pending_value = Some(value);
        match self.key_serializer.input(key, sink)? {
            Poll::Pending => {
                self.key_started = true;
                Ok(Poll::Pending)
            }
            Poll::Ready(()) => {
                self.key_started = true;
                self.advance_after_key(sink)
            }
        }
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        if self.stage == 0 {
            if !self.key_started {
                return Err(ZebinError::SerializationError {
                    pos: sink.pos(),
                    message: "Tuple2Serializer polled before input",
                });
            }
            match self.key_serializer.poll_pending(sink)? {
                Poll::Pending => return Ok(Poll::Pending),
                Poll::Ready(()) => {}
            }
            return self.advance_after_key(sink);
        }
        if self.stage == 1 {
            if !self.value_started {
                let v = self
                    .pending_value
                    .take()
                    .ok_or(ZebinError::SerializationError {
                        pos: sink.pos(),
                        message: "Tuple2Serializer lost pending value",
                    })?;
                self.value_started = true;
                match self.value_serializer.input(v, sink)? {
                    Poll::Pending => return Ok(Poll::Pending),
                    Poll::Ready(()) => {}
                }
            } else {
                match self.value_serializer.poll_pending(sink)? {
                    Poll::Pending => return Ok(Poll::Pending),
                    Poll::Ready(()) => {}
                }
            }
            self.stage = 2;
        }
        Ok(Poll::Ready(()))
    }

    fn finish<S: StorageMut + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
    }
}

impl<'a, K, V> Tuple2Serializer<'a, K, V>
where
    K: Serialize<Input<'a> = K> + Archive + 'a,
    V: Serialize<Input<'a> = V> + Archive + 'a,
{
    fn advance_after_key<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.stage = 1;
        let v = self
            .pending_value
            .take()
            .ok_or(ZebinError::SerializationError {
                pos: sink.pos(),
                message: "Tuple2Serializer lost pending value",
            })?;
        self.value_started = true;
        match self.value_serializer.input(v, sink)? {
            Poll::Pending => Ok(Poll::Pending),
            Poll::Ready(()) => {
                self.stage = 2;
                Ok(Poll::Ready(()))
            }
        }
    }
}

impl<K: Archive, V: Archive> Archive for (K, V) {
    type Archived = (K::Archived, V::Archived);
}

impl<K, V> Serialize for (K, V)
where
    K: Serialize + Archive,
    V: Serialize + Archive,
    for<'a> K: Serialize<Input<'a> = K> + 'a,
    for<'a> V: Serialize<Input<'a> = V> + 'a,
{
    type Input<'a>
        = (K, V)
    where
        Self: 'a;
    type Serializer<'a>
        = Tuple2Serializer<'a, K, V>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        Tuple2Serializer::new()
    }
}

impl<K, V> MeasureBody for (K, V)
where
    K: MeasureBody,
    V: MeasureBody,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        self.0
            .measure_body()?
            .checked_add(self.1.measure_body()?)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })
    }
}

impl<A: ArchivedLayout, B: ArchivedLayout> ArchivedLayout for (A, B) {
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
}

impl<A: Access, B: Access> Access for (A, B) {
    type View<'a2>
        = (A::View<'a2>, B::View<'a2>)
    where
        Self: 'a2;
    #[cfg(feature = "alloc")]
    type AccessStrategy = crate::io::ForwardSequenceStrategy;

    fn access<'a2, C>(
        cursor: &mut Cursor<'a2>,
        context: &mut C,
    ) -> Result<Self::View<'a2>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a2,
    {
        let key = A::access(cursor, context)?;
        let value = B::access(cursor, context)?;
        Ok((key, value))
    }

    fn validate<'a2, C>(cursor: &mut Cursor<'a2>, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
    {
        A::validate(cursor, context)?;
        B::validate(cursor, context)?;
        Ok(())
    }
}

impl<A, B, U, V> Deserialize<(U, V)> for (A, B)
where
    A: Deserialize<U>,
    B: Deserialize<V>,
{
    fn deserialize(&self) -> Result<(U, V), ZebinError> {
        Ok((self.0.deserialize()?, self.1.deserialize()?))
    }
}

impl<'a, A, B> ArchivedField<'a> for (A, B)
where
    A: ArchivedField<'a>,
    B: ArchivedField<'a>,
{
}
