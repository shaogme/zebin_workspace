use core::task::Poll;

#[cfg(feature = "alloc")]
use crate::io::ForwardSequenceStrategy;
use crate::prelude::*;

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

/// Accessd representation for `Option<T>`.
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

impl<A> Access for ArchivedOption<A>
where
    A: Access,
{
    type View<'a>
        = ArchivedOption<A::View<'a>>
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
        let pos = cursor.pos();
        match cursor.read_u8(context)? {
            0 => Ok(ArchivedOption::None),
            1 => {
                let mut guard = context.push_variant("Some");
                Ok(ArchivedOption::Some(A::access(cursor, &mut *guard)?))
            }
            _ => Err(context.validation_error("Invalid Option discriminant", pos)),
        }
    }

    fn validate<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
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

impl<T, U> Deserialize<Option<U>> for ArchivedOption<T>
where
    T: Deserialize<U>,
{
    fn deserialize(&self) -> Result<Option<U>, ZebinError> {
        match self {
            ArchivedOption::Some(value) => Ok(value.deserialize().map(Some)?),
            ArchivedOption::None => Ok(None),
        }
    }

    fn deserialize_missing() -> Result<Option<U>, ZebinError> {
        Ok(None)
    }
}

impl<T, U> Deserialize<Option<U>> for Option<T>
where
    T: Deserialize<U>,
{
    fn deserialize(&self) -> Result<Option<U>, ZebinError> {
        match self {
            Some(value) => Ok(Some(value.deserialize()?)),
            None => Ok(None),
        }
    }
}

/// Resumable serialization state for `Option<T>`.
pub struct OptionSerializer<'a, T>
where
    T: Serialize + Archive + 'a,
{
    value: Option<T>,
    prefix: [u8; 1],
    prefix_cursor: usize,
    inner: <T as Serialize>::Serializer<'a>,
    inner_started: bool,
    has_inner: bool,
}

impl<'a, T> OptionSerializer<'a, T>
where
    T: Serialize + Archive + 'a,
{
    pub(crate) fn new_empty() -> Self {
        Self {
            value: None,
            prefix: [0],
            prefix_cursor: 1,
            inner: T::serializer(),
            inner_started: false,
            has_inner: false,
        }
    }
}

impl<'a, T> Serializer for OptionSerializer<'a, T>
where
    T: Serialize<Input<'a> = T> + Archive + 'a,
{
    type Input = Option<T>;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut dyn CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        match item {
            Some(inner) => {
                self.value = Some(inner);
                self.prefix = [1];
                self.prefix_cursor = 0;
                self.has_inner = true;
                self.inner_started = false;
            }
            None => {
                self.value = None;
                self.prefix = [0];
                self.prefix_cursor = 0;
                self.has_inner = false;
                self.inner_started = false;
            }
        }
        self.poll_pending(sink)
    }

    fn poll_pending(&mut self, sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        if self.prefix_cursor < self.prefix.len() {
            let remaining = self.prefix.len() - self.prefix_cursor;
            if sink
                .write(&self.prefix[self.prefix_cursor..])?
                .advance_cursor(&mut self.prefix_cursor, remaining)
                .is_pending()
            {
                return Ok(Poll::Pending);
            }
        }

        if self.has_inner {
            let progress = if !self.inner_started {
                let item = self.value.take().expect("must be Some if has_inner");
                self.inner_started = true;
                self.inner.input(item, sink)?
            } else {
                self.inner.poll_pending(sink)?
            };

            match progress {
                Poll::Pending => Ok(Poll::Pending),
                Poll::Ready(()) => {
                    self.has_inner = false;
                    Ok(Poll::Ready(()))
                }
            }
        } else {
            Ok(Poll::Ready(()))
        }
    }

    fn finish(self, sink: &mut dyn CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<T> Archive for Option<T>
where
    T: Archive,
{
    type Archived = ArchivedOption<T::Archived>;
    const ALLOW_MISSING: bool = true;
}

impl<T> Serialize for Option<T>
where
    T: Serialize + Archive,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = Option<T>
    where
        Self: 'a;
    type Serializer<'a>
        = OptionSerializer<'a, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        OptionSerializer::new_empty()
    }
}

impl<T> MeasureBody for Option<T>
where
    T: MeasureBody,
{
    fn measure_body(&self) -> Result<usize, ZebinError> {
        match self {
            Some(v) => Ok(1usize
                .checked_add(v.measure_body()?)
                .ok_or(ZebinError::ArithmeticOverflow { pos: 0 })?),
            None => Ok(1),
        }
    }
}

impl<'a, T: 'a> ArchivedField<'a> for ArchivedOption<T> {
    #[inline]
    fn resolve_field(
        view: Option<&Self>,
        _field_id: u16,
        _pos: usize,
    ) -> Result<&Self, ZebinError> {
        match view {
            Some(v) => Ok(v),
            None => Ok(&ArchivedOption::None),
        }
    }
}
