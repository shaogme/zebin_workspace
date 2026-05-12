use core::task::Poll;

use crate::{
    core::schema::ObjectEncoding,
    error::{DecodeError, ZebinError},
    traits::{
        Archive, ArchivedDefault, ArchivedLayout, ByteSink, Decode, Encode, EncodeState, Restore,
        SchemaAware,
    },
    validation::context::ValidationContext,
};

impl<T> SchemaAware for ArchivedOption<T> {
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

/// Decoded representation for `Option<T>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArchivedOption<T> {
    None,
    Some(T),
}

impl<T> ArchivedOption<T> {
    pub fn is_some(&self) -> bool {
        matches!(self, Self::Some(_))
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Some(value) => Some(value),
            Self::None => None,
        }
    }
}

impl<A> ArchivedLayout for ArchivedOption<A>
where
    A: ArchivedLayout,
{
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Sequence;
}

impl<'a, A> Decode<'a> for ArchivedOption<A>
where
    A: Decode<'a>,
{
    type View = ArchivedOption<A::View>;

    fn decode<C>(
        cursor: &mut crate::read::Cursor<'a>,
        context: &mut C,
    ) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        match cursor.read_u8(context)? {
            0 => Ok(ArchivedOption::None),
            1 => {
                let mut guard = context.push_variant("Some");
                Ok(ArchivedOption::Some(A::decode(cursor, &mut *guard)?))
            }
            _ => Err(context.validation_error("Invalid Option discriminant", pos)),
        }
    }

    fn validate<C>(cursor: &mut crate::read::Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized,
    {
        let pos = cursor.pos();
        match cursor.read_u8(context)? {
            0 => Ok(()),
            1 => {
                let mut guard = context.push_variant("Some");
                A::validate(cursor, &mut *guard)
            }
            _ => Err(context.validation_error("Invalid Option discriminant", pos)),
        }
    }
}

impl<T: 'static> ArchivedDefault for ArchivedOption<T> {
    fn archived_default() -> &'static Self {
        static DEFAULT: ArchivedOption<()> = ArchivedOption::None;
        unsafe { &*(&DEFAULT as *const ArchivedOption<()> as *const ArchivedOption<T>) }
    }
}

impl<T, U> Restore<Option<U>> for ArchivedOption<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Option<U>, ZebinError> {
        match self {
            ArchivedOption::Some(value) => Ok(Some(value.restore()?)),
            ArchivedOption::None => Ok(None),
        }
    }
}

impl<T, U> Restore<Option<U>> for Option<T>
where
    T: Restore<U>,
{
    fn restore(&self) -> Result<Option<U>, ZebinError> {
        match self {
            Some(value) => Ok(Some(value.restore()?)),
            None => Ok(None),
        }
    }
}

/// Resumable serialization state for `Option<T>`.
pub struct OptionArchiveState<'a, T>
where
    T: Encode + Archive + 'a,
{
    prefix: [u8; 1],
    prefix_cursor: usize,
    inner: Option<<T as Encode>::State<'a>>,
}

impl<'a, T> OptionArchiveState<'a, T>
where
    T: Encode + Archive + 'a,
{
    fn new(value: Option<&'a T>) -> Result<Self, ZebinError> {
        match value {
            Some(inner) => Ok(Self {
                prefix: [1],
                prefix_cursor: 0,
                inner: Some(inner.begin_encode()?),
            }),
            None => Ok(Self {
                prefix: [0],
                prefix_cursor: 0,
                inner: None,
            }),
        }
    }
}

impl<'a, T> EncodeState<'a> for OptionArchiveState<'a, T>
where
    T: Encode + Archive + 'a,
{
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.prefix.len() {
            let remaining = self.prefix.len() - self.prefix_cursor;
            if encoder
                .write(&self.prefix[self.prefix_cursor..])?
                .advance_cursor(&mut self.prefix_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

        if let Some(inner) = &mut self.inner {
            match inner.poll(encoder)? {
                Poll::Pending => Ok(Poll::Pending),
                Poll::Ready(()) => {
                    self.inner = None;
                    Ok(Poll::Ready(()))
                }
            }
        } else {
            Ok(Poll::Ready(()))
        }
    }
}

impl<T> Archive for Option<T>
where
    T: Archive,
{
    type Archived = ArchivedOption<T::Archived>;
}

impl<T> Encode for Option<T>
where
    T: Encode + Archive,
{
    type State<'a>
        = OptionArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::State<'_>, ZebinError> {
        OptionArchiveState::new(self.as_ref())
    }
}
