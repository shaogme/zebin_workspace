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

/// Accessd representation for `Result<T, E>`.
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

impl<A, B> Access for ArchivedResult<A, B>
where
    A: Access,
    B: Access,
{
    type View<'a>
        = ArchivedResult<A::View<'a>, B::View<'a>>
    where
        Self: 'a;
    #[cfg(feature = "alloc")]
    type AccessStrategy = ForwardSequenceStrategy;

    fn access<'a, C>(
        cursor: &mut Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Self: 'a,
    {
        let pos = cursor.pos();
        match cursor.read_u8(context)? {
            0 => {
                let mut guard = context.push_variant("Ok");
                Ok(ArchivedResult::Ok(A::access(cursor, &mut *guard)?))
            }
            1 => {
                let mut guard = context.push_variant("Err");
                Ok(ArchivedResult::Err(B::access(cursor, &mut *guard)?))
            }
            _ => Err(context.validation_error("Invalid Result discriminant", pos)),
        }
    }

    fn validate<'a, C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), AccessError>
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

impl<T, E, U, V> Deserialize<Result<U, V>> for ArchivedResult<T, E>
where
    T: Deserialize<U>,
    E: Deserialize<V>,
{
    fn deserialize(&self) -> Result<Result<U, V>, ZebinError> {
        match self {
            ArchivedResult::Ok(value) => Ok(Ok(value.deserialize()?)),
            ArchivedResult::Err(value) => Ok(Err(value.deserialize()?)),
        }
    }
}

impl<T, E, U, V> Deserialize<Result<U, V>> for Result<T, E>
where
    T: Deserialize<U>,
    E: Deserialize<V>,
{
    fn deserialize(&self) -> Result<Result<U, V>, ZebinError> {
        match self {
            Ok(value) => Ok(Ok(value.deserialize()?)),
            Err(value) => Ok(Err(value.deserialize()?)),
        }
    }
}

pub struct ResultSerializer<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    state: ResultSerializerState<T, E>,
    ok_serializer: <T as Serialize>::Serializer<'a>,
    err_serializer: <E as Serialize>::Serializer<'a>,
}

enum ResultSerializerState<T, E> {
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

impl<'a, T, E> ResultSerializer<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    pub(crate) fn new_empty() -> Self {
        Self {
            state: ResultSerializerState::Uninitialized,
            ok_serializer: T::serializer(),
            err_serializer: E::serializer(),
        }
    }
}

impl<'a, T, E> Serializer for ResultSerializer<'a, T, E>
where
    T: Serialize<Input<'a> = T> + Archive + 'a,
    E: Serialize<Input<'a> = E> + Archive + 'a,
{
    type Input = Result<T, E>;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        self.state = match item {
            Ok(inner) => ResultSerializerState::Ok {
                val: Some(inner),
                prefix_cursor: 0,
                started: false,
            },
            Err(inner) => ResultSerializerState::Err {
                val: Some(inner),
                prefix_cursor: 0,
                started: false,
            },
        };
        self.poll_pending(sink)
    }

    fn poll_pending(&mut self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        match &mut self.state {
            ResultSerializerState::Uninitialized => Ok(Poll::Ready(())),
            ResultSerializerState::Ok {
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
                    let v = val.take().ok_or(ZebinError::SerializeError {
                        pos: sink.pos(),
                        message: "ResultSerializer lost Ok value",
                    })?;
                    *started = true;
                    self.ok_serializer.input(v, sink)?
                } else {
                    self.ok_serializer.poll_pending(sink)?
                };

                Ok(progress)
            }
            ResultSerializerState::Err {
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
                    let v = val.take().ok_or(ZebinError::SerializeError {
                        pos: sink.pos(),
                        message: "ResultSerializer lost Err value",
                    })?;
                    *started = true;
                    self.err_serializer.input(v, sink)?
                } else {
                    self.err_serializer.poll_pending(sink)?
                };

                Ok(progress)
            }
        }
    }

    fn finish(self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        match self.state {
            ResultSerializerState::Uninitialized => Ok(Poll::Ready(())),
            ResultSerializerState::Ok { .. } => self.ok_serializer.finish(sink),
            ResultSerializerState::Err { .. } => self.err_serializer.finish(sink),
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

impl<T, E> Serialize for Result<T, E>
where
    T: Serialize + Archive,
    E: Serialize + Archive,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
    for<'a> E: Serialize<Input<'a> = E> + 'a,
{
    type Input<'a>
        = Result<T, E>
    where
        Self: 'a;
    type Serializer<'a>
        = ResultSerializer<'a, T, E>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        ResultSerializer::new_empty()
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
