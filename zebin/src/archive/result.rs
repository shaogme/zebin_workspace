#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::prelude::*;
use core::task::Poll;

impl<T, E> SchemaAware for ArchivedResult<T, E> {
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

/// Decoded representation for `Result<T, E>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchivedResult<T, E> {
    Ok(T),
    Err(E),
}

impl<T, E> ArchivedResult<T, E> {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    pub fn is_err(&self) -> bool {
        matches!(self, Self::Err(_))
    }

    pub fn as_ok(&self) -> Option<&T> {
        match self {
            Self::Ok(value) => Some(value),
            Self::Err(_) => None,
        }
    }

    pub fn as_err(&self) -> Option<&E> {
        match self {
            Self::Ok(_) => None,
            Self::Err(value) => Some(value),
        }
    }
}

impl<A, B> ArchivedLayout for ArchivedResult<A, B>
where
    A: ArchivedLayout,
    B: ArchivedLayout,
{
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
}

impl<'a, A, B> Decode<'a> for ArchivedResult<A, B>
where
    A: Decode<'a>,
    B: Decode<'a>,
{
    type View = ArchivedResult<A::View, B::View>;
    #[cfg(feature = "alloc")]
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        match cursor.read_u8(context)? {
            0 => {
                let mut guard = context.push_variant("Ok");
                Ok(ArchivedResult::Ok(A::decode(cursor, &mut *guard)?))
            }
            1 => {
                let mut guard = context.push_variant("Err");
                Ok(ArchivedResult::Err(B::decode(cursor, &mut *guard)?))
            }
            _ => Err(context.validation_error("Invalid Result discriminant", pos)),
        }
    }

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        match cursor.read_u8(context)? {
            0 => {
                let mut guard = context.push_variant("Ok");
                A::validate(cursor, &mut *guard)
            }
            1 => {
                let mut guard = context.push_variant("Err");
                B::validate(cursor, &mut *guard)
            }
            _ => Err(context.validation_error("Invalid Result discriminant", pos)),
        }
    }
}

impl<T, E, U, V> Restore<Result<U, V>> for ArchivedResult<T, E>
where
    T: Restore<U>,
    E: Restore<V>,
{
    fn restore(&self) -> Result<Result<U, V>, ZebinError> {
        match self {
            ArchivedResult::Ok(value) => Ok(Ok(value.restore()?)),
            ArchivedResult::Err(value) => Ok(Err(value.restore()?)),
        }
    }
}

impl<T, E, U, V> Restore<Result<U, V>> for Result<T, E>
where
    T: Restore<U>,
    E: Restore<V>,
{
    fn restore(&self) -> Result<Result<U, V>, ZebinError> {
        match self {
            Ok(value) => Ok(Ok(value.restore()?)),
            Err(value) => Ok(Err(value.restore()?)),
        }
    }
}

pub struct ResultEncoder<'a, T, E>
where
    T: Encode + Archive + 'a,
    E: Encode + Archive + 'a,
{
    state: ResultEncoderState<T, E>,
    ok_encoder: <T as Encode>::Encoder<'a>,
    err_encoder: <E as Encode>::Encoder<'a>,
}

enum ResultEncoderState<T, E> {
    Uninitialized,
    Ok {
        val: Option<T>,
        prefix_cursor: usize,
        started: bool,
    },
    Err {
        val: Option<E>,
        prefix_cursor: usize,
        started: bool,
    },
}

impl<'a, T, E> ResultEncoder<'a, T, E>
where
    T: Encode + Archive + 'a,
    E: Encode + Archive + 'a,
{
    pub(crate) fn new_empty() -> Self {
        Self {
            state: ResultEncoderState::Uninitialized,
            ok_encoder: T::encoder(),
            err_encoder: E::encoder(),
        }
    }
}

impl<'a, T, E> Encoder for ResultEncoder<'a, T, E>
where
    T: Encode<Input<'a> = T> + Archive + 'a,
    E: Encode<Input<'a> = E> + Archive + 'a,
{
    type Input = Result<T, E>;

    fn input<S: StorageMut + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.state = match item {
            Ok(inner) => ResultEncoderState::Ok {
                val: Some(inner),
                prefix_cursor: 0,
                started: false,
            },
            Err(inner) => ResultEncoderState::Err {
                val: Some(inner),
                prefix_cursor: 0,
                started: false,
            },
        };
        self.poll_pending(sink)
    }

    fn poll_pending<S: StorageMut + ?Sized>(
        &mut self,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        match &mut self.state {
            ResultEncoderState::Uninitialized => Ok(Poll::Ready(())),
            ResultEncoderState::Ok {
                val,
                prefix_cursor,
                started,
            } => {
                if *prefix_cursor == 0
                    && sink
                        .write(&[0])?
                        .advance_cursor(prefix_cursor, 1)
                        .is_pending()
                {
                    return Ok(Poll::Pending);
                }

                let progress = if !*started {
                    let v = val.take().ok_or(ZebinError::SerializationError {
                        pos: sink.pos(),
                        message: "ResultEncoder lost Ok value",
                    })?;
                    *started = true;
                    self.ok_encoder.input(v, sink)?
                } else {
                    self.ok_encoder.poll_pending(sink)?
                };

                Ok(progress)
            }
            ResultEncoderState::Err {
                val,
                prefix_cursor,
                started,
            } => {
                if *prefix_cursor == 0
                    && sink
                        .write(&[1])?
                        .advance_cursor(prefix_cursor, 1)
                        .is_pending()
                {
                    return Ok(Poll::Pending);
                }

                let progress = if !*started {
                    let v = val.take().ok_or(ZebinError::SerializationError {
                        pos: sink.pos(),
                        message: "ResultEncoder lost Err value",
                    })?;
                    *started = true;
                    self.err_encoder.input(v, sink)?
                } else {
                    self.err_encoder.poll_pending(sink)?
                };

                Ok(progress)
            }
        }
    }

    fn finish<S: StorageMut + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        match self.state {
            ResultEncoderState::Uninitialized => Ok(Poll::Ready(())),
            ResultEncoderState::Ok { .. } => self.ok_encoder.finish(sink),
            ResultEncoderState::Err { .. } => self.err_encoder.finish(sink),
        }
    }
}

impl<T, E> Archive for Result<T, E>
where
    T: Archive,
    E: Archive,
{
    type Archived = ArchivedResult<T::Archived, E::Archived>;
}

impl<T, E> Encode for Result<T, E>
where
    T: Encode + Archive,
    E: Encode + Archive,
    for<'a> T: Encode<Input<'a> = T> + 'a,
    for<'a> E: Encode<Input<'a> = E> + 'a,
{
    type Input<'a>
        = Result<T, E>
    where
        Self: 'a;
    type Encoder<'a>
        = ResultEncoder<'a, T, E>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        ResultEncoder::new_empty()
    }
}

impl<T, E> MeasureBody for Result<T, E>
where
    T: MeasureBody,
    E: MeasureBody,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        let inner = match self {
            Ok(v) => v.measure_body()?,
            Err(v) => v.measure_body()?,
        };
        1usize
            .checked_add(inner)
            .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })
    }
}

impl<'a, T: 'a, E: 'a> ArchivedField<'a> for ArchivedResult<T, E> {}
