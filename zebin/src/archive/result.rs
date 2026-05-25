use core::task::Poll;

use crate::{
    core::schema::ObjectEncoding,
    error::{DecodeError, ZebinError},
    traits::{Archive, ArchivedLayout, ByteSink, Decode, Encode, Encoder, Restore, SchemaAware},
    validation::context::ValidationContext,
};

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
    type DecodeStrategy = crate::traits::ForwardSequenceStrategy;

    fn decode<C>(
        cursor: &mut crate::read::Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View, DecodeError>
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

    fn validate<C>(cursor: &mut crate::read::Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
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

/// Resumable serialization state for `Result<T, E>`.
pub enum ResultEncoder<'a, T, E>
where
    T: Encode + Archive + 'a,
    E: Encode + Archive + 'a,
{
    Ok {
        val: &'a T,
        prefix_cursor: usize,
        encoder: <T as Encode>::Encoder<'a>,
        started: bool,
    },
    Err {
        val: &'a E,
        prefix_cursor: usize,
        encoder: <E as Encode>::Encoder<'a>,
        started: bool,
    },
}

impl<'a, T, E> ResultEncoder<'a, T, E>
where
    T: Encode + Archive + 'a,
    E: Encode + Archive + 'a,
{
    pub(crate) fn new(value: Result<&'a T, &'a E>) -> Result<Self, ZebinError> {
        match value {
            Ok(inner) => Ok(Self::Ok {
                val: inner,
                prefix_cursor: 0,
                encoder: inner.begin_encode()?,
                started: false,
            }),
            Err(inner) => Ok(Self::Err {
                val: inner,
                prefix_cursor: 0,
                encoder: inner.begin_encode()?,
                started: false,
            }),
        }
    }
}

impl<'a, T, E> Encoder<'a> for ResultEncoder<'a, T, E>
where
    T: Encode + Archive + 'a,
    E: Encode + Archive + 'a,
{
    type Input = &'a Result<T, E>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        _item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        match self {
            ResultEncoder::Ok {
                val,
                prefix_cursor,
                encoder,
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
                    match encoder.input(*val, sink)? {
                        Poll::Pending => {
                            *started = true;
                            Poll::Pending
                        }
                        Poll::Ready(()) => Poll::Ready(()),
                    }
                } else {
                    encoder.poll_pending(sink)?
                };

                match progress {
                    Poll::Pending => Ok(Poll::Pending),
                    Poll::Ready(()) => Ok(Poll::Ready(())),
                }
            }
            ResultEncoder::Err {
                val,
                prefix_cursor,
                encoder,
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
                    match encoder.input(*val, sink)? {
                        Poll::Pending => {
                            *started = true;
                            Poll::Pending
                        }
                        Poll::Ready(()) => Poll::Ready(()),
                    }
                } else {
                    encoder.poll_pending(sink)?
                };

                match progress {
                    Poll::Pending => Ok(Poll::Pending),
                    Poll::Ready(()) => Ok(Poll::Ready(())),
                }
            }
        }
    }

    fn finish<S: ByteSink + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
        match self {
            ResultEncoder::Ok { encoder, .. } => encoder.finish(sink),
            ResultEncoder::Err { encoder, .. } => encoder.finish(sink),
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
{
    type Encoder<'a>
        = ResultEncoder<'a, T, E>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError> {
        ResultEncoder::new(self.as_ref())
    }
}
