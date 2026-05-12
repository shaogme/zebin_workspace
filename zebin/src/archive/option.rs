use core::task::Poll;

use crate::{
    core::schema::ObjectEncoding,
    error::{AccessError, ZebinError},
    traits::{
        Archive, ArchivedDefault, ByteSink, Decode, Restore, SchemaAware, Serialize, SerializeState,
    },
    validation::context::ValidationContext,
};

impl<T> SchemaAware for ArchivedOption<T> {
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

impl<'a, A> Decode<'a> for ArchivedOption<A>
where
    A: Decode<'a>,
{
    type View = ArchivedOption<A::View>;
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
            0 => Ok(ArchivedOption::None),
            1 => {
                let mut guard = context.push_variant("Some");
                Ok(ArchivedOption::Some(A::decode(cursor, &mut *guard)?))
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
    T: Serialize + Archive + 'a,
{
    prefix: [u8; 1],
    prefix_cursor: usize,
    inner: Option<<T as Serialize>::State<'a>>,
}

impl<'a, T> OptionArchiveState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    fn new(value: Option<&'a T>) -> Result<Self, ZebinError> {
        match value {
            Some(inner) => Ok(Self {
                prefix: [1],
                prefix_cursor: 0,
                inner: Some(inner.begin_serialize()?),
            }),
            None => Ok(Self {
                prefix: [0],
                prefix_cursor: 0,
                inner: None,
            }),
        }
    }
}

impl<'a, T> SerializeState<'a> for OptionArchiveState<'a, T>
where
    T: Serialize + Archive + 'a,
{
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.prefix.len() {
            let written = encoder.write(&self.prefix[self.prefix_cursor..])?;
            self.prefix_cursor += written;
            if self.prefix_cursor < self.prefix.len() {
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

impl<T> Serialize for Option<T>
where
    T: Serialize + Archive,
{
    type State<'a>
        = OptionArchiveState<'a, T>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError> {
        OptionArchiveState::new(self.as_ref())
    }
}
