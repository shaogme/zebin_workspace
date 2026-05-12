use core::task::Poll;

use crate::{
    core::schema::ObjectEncoding,
    error::{AccessError, ZebinError},
    traits::{Archive, ByteSink, Decode, Restore, SchemaAware, Serialize, SerializeState},
    validation::context::ValidationContext,
};

impl<T, E> SchemaAware for ArchivedResult<T, E> {
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

impl<'a, A, B> Decode<'a> for ArchivedResult<A, B>
where
    A: Decode<'a>,
    B: Decode<'a>,
{
    type View = ArchivedResult<A::View, B::View>;
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;

    fn decode<C>(
        cursor: &mut crate::read::Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View, AccessError>
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

/// Resumable serialization state for `Result<T, E>`.
pub enum ResultArchiveState<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    Ok {
        prefix_cursor: usize,
        state: <T as Serialize>::State<'a>,
    },
    Err {
        prefix_cursor: usize,
        state: <E as Serialize>::State<'a>,
    },
}

impl<'a, T, E> ResultArchiveState<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    pub(crate) fn new(value: Result<&'a T, &'a E>) -> Result<Self, ZebinError> {
        match value {
            Ok(inner) => Ok(Self::Ok {
                prefix_cursor: 0,
                state: inner.begin_serialize()?,
            }),
            Err(inner) => Ok(Self::Err {
                prefix_cursor: 0,
                state: inner.begin_serialize()?,
            }),
        }
    }
}

impl<'a, T, E> SerializeState<'a> for ResultArchiveState<'a, T, E>
where
    T: Serialize + Archive + 'a,
    E: Serialize + Archive + 'a,
{
    fn poll<R: ByteSink + ?Sized>(&mut self, encoder: &mut R) -> Result<Poll<()>, ZebinError> {
        match self {
            ResultArchiveState::Ok {
                prefix_cursor,
                state,
            } => {
                if *prefix_cursor == 0 {
                    let written = encoder.write(&[0])?;
                    *prefix_cursor += written;
                    if *prefix_cursor == 0 {
                        return Ok(Poll::Pending);
                    }
                }
                state.poll(encoder)
            }
            ResultArchiveState::Err {
                prefix_cursor,
                state,
            } => {
                if *prefix_cursor == 0 {
                    let written = encoder.write(&[1])?;
                    *prefix_cursor += written;
                    if *prefix_cursor == 0 {
                        return Ok(Poll::Pending);
                    }
                }
                state.poll(encoder)
            }
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
{
    type State<'a>
        = ResultArchiveState<'a, T, E>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        ResultArchiveState::new(self.as_ref())
    }
}
