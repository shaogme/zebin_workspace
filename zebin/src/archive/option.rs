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
    #[cfg(feature = "alloc")]
    type DecodeStrategy = ForwardSequenceStrategy;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
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

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
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
pub struct OptionEncoder<'a, T>
where
    T: Encode + Archive + 'a,
{
    value: Option<&'a T>,
    prefix: [u8; 1],
    prefix_cursor: usize,
    inner: Option<(<T as Encode>::Encoder<'a>, bool)>,
}

impl<'a, T> OptionEncoder<'a, T>
where
    T: Encode + Archive + 'a,
{
    pub(crate) fn new_empty() -> Self {
        Self {
            value: None,
            prefix: [0],
            prefix_cursor: 1,
            inner: None,
        }
    }
}

impl<'a, T> Encoder<'a> for OptionEncoder<'a, T>
where
    T: Encode + Archive + 'a,
{
    type Input = &'a Option<T>;

    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError> {
        match item {
            Some(inner) => {
                self.value = Some(inner);
                self.prefix = [1];
                self.prefix_cursor = 0;
                self.inner = Some((T::encoder(), false));
            }
            None => {
                self.value = None;
                self.prefix = [0];
                self.prefix_cursor = 0;
                self.inner = None;
            }
        }
        self.poll_pending(sink)
    }

    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError> {
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

        if let Some((encoder, started)) = &mut self.inner {
            let progress = if !*started {
                let item = self.value.expect("must be Some if inner is Some");
                match encoder.input(item, sink)? {
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
                Poll::Ready(()) => {
                    let (encoder, _) = self.inner.take().expect("present");
                    let _ = encoder.finish(sink)?;
                    Ok(Poll::Ready(()))
                }
            }
        } else {
            Ok(Poll::Ready(()))
        }
    }

    fn finish<S: ByteSink + ?Sized>(self, _sink: &mut S) -> Result<Poll<()>, ZebinError> {
        Ok(Poll::Ready(()))
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
    type Encoder<'a>
        = OptionEncoder<'a, T>
    where
        Self: 'a;

    fn encoder<'a>() -> Self::Encoder<'a>
    where
        Self: 'a,
    {
        OptionEncoder::new_empty()
    }
}
