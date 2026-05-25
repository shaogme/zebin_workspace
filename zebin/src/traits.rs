use core::{num::NonZeroUsize, task::Poll};

use crate::{
    core::schema::{FieldEncoding, ObjectEncoding},
    error::{DecodeError, ParseHeaderError, ZebinError},
    read::Cursor,
    validation::context::ValidationContext,
};

/// Fixed-width archived overlay contract.
///
/// Only plain fixed-size overlays implement this trait. Variable-width archive
/// forms are decoded through [`Decode`] instead of pretending to have a stable
/// in-place layout.
pub trait FixedLayout: Sized {
    const ALIGNMENT: NonZeroUsize;
    const SIZE: usize = core::mem::size_of::<Self>();

    fn write_fixed(archived: &Self, out: &mut [u8]);
}

/// Archive format header contract.
pub trait ArchiveHeader: Clone + Copy {
    type Bytes: AsRef<[u8]> + Copy + Send + Sync;

    const MAGIC: [u8; 2];
    const VERSION: u8;
    const SIZE: usize;

    fn parse(bytes: &[u8]) -> Result<Self, ParseHeaderError>;

    fn flags(&self) -> u8;

    fn encode(&self) -> Self::Bytes;

    fn create(flags: u8) -> Self;
}

/// Static archived layout metadata shared by read and write paths.
pub trait ArchivedLayout {
    const FIXED_SIZE: Option<usize> = None;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Fixed;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::Fixed;
}

/// Read-side decode contract for consuming a value from a sequential cursor.
pub trait Decode<'a>: ArchivedLayout + Sized {
    type View: 'a;

    fn decode<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<Self::View, DecodeError>
    where
        C: ValidationContext + ?Sized;

    fn validate<C>(cursor: &mut Cursor<'a>, context: &mut C) -> Result<(), DecodeError>
    where
        C: ValidationContext + ?Sized;
}

/// Object model layer: type-level archive and decode contracts.
pub trait Archive {
    type Archived;
}

/// Contract for providing a static default archived view value.
pub trait ArchivedDefault {
    fn archived_default() -> &'static Self;
}

/// Contract for schema-aware decoded views.
pub trait SchemaAware {
    fn pos(&self) -> usize;
    fn stable_schema_key(&self) -> u32;
    fn schema_revision(&self) -> u32;
}

impl<T: SchemaAware + ?Sized> SchemaAware for &T {
    fn pos(&self) -> usize {
        (**self).pos()
    }

    fn stable_schema_key(&self) -> u32 {
        (**self).stable_schema_key()
    }

    fn schema_revision(&self) -> u32 {
        (**self).schema_revision()
    }
}

/// Contract for decoded views that can restore the source type.
pub trait Restore<T> {
    fn restore(&self) -> Result<T, ZebinError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkProgress {
    Complete,
    Partial(NonZeroUsize),
    Blocked,
}

impl SinkProgress {
    pub fn from_accepted(requested: usize, accepted: usize) -> Self {
        debug_assert!(accepted <= requested);
        if accepted == requested {
            Self::Complete
        } else if let Some(accepted) = NonZeroUsize::new(accepted) {
            Self::Partial(accepted)
        } else {
            Self::Blocked
        }
    }

    pub fn accepted_for(self, requested: usize) -> usize {
        match self {
            Self::Complete => requested,
            Self::Partial(accepted) => accepted.get(),
            Self::Blocked => 0,
        }
    }

    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }

    pub fn advance_cursor(self, cursor: &mut usize, requested: usize) -> Poll<()> {
        *cursor += self.accepted_for(requested);
        if self.is_complete() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

/// Byte-stream sink used by archive state machines.
pub trait ByteSink {
    /// Returns the current absolute position in the archive being written.
    fn pos(&self) -> usize;

    /// Attempts to write the provided bytes into the sink.
    fn write(&mut self, bytes: &[u8]) -> Result<SinkProgress, ZebinError>;

    /// Attempts to align the current archive position to the specified alignment.
    fn align(&mut self, alignment: NonZeroUsize) -> Result<SinkProgress, ZebinError>;

    /// Attempts to skip (fill with zeros) the specified number of bytes.
    fn skip(&mut self, len: usize) -> Result<SinkProgress, ZebinError>;
}

/// Unified encoder protocol, supporting one-off or incremental step-by-step input.
pub trait Encoder<'a> {
    /// The type of input items received by the encoder.
    /// - For one-off encoding, this can be `()` (since data is already bound at creation).
    /// - For streamable step-by-step encoding, this is the concrete input slice/item type (e.g., `&'a T`, `&'a [u8]`, etc.).
    type Input;

    /// Attempts to input a data slice into the encoder and encode it into the underlying `ByteSink`.
    ///
    /// # Return Value
    /// - `Ok(Poll::Ready(()))`: The current input item has been fully encoded and written.
    /// - `Ok(Poll::Pending)`: The `ByteSink` is full. The current input item is pending, and the caller should flush/provide a new Sink and call `poll_pending`.
    fn input<S: ByteSink + ?Sized>(
        &mut self,
        item: Self::Input,
        sink: &mut S,
    ) -> Result<Poll<()>, ZebinError>;

    /// Advances and flushes any state/data previously accumulated inside the encoder due to insufficient `ByteSink` space.
    ///
    /// Regardless of whether it is a one-off or step-by-step input, this method can be called to advance the remaining encoding progress until it returns `Poll::Ready(())`.
    fn poll_pending<S: ByteSink + ?Sized>(&mut self, sink: &mut S) -> Result<Poll<()>, ZebinError>;

    /// Finishes the encoding process, writing any necessary alignments, paddings, or trailing metadata.
    fn finish<S: ByteSink + ?Sized>(self, sink: &mut S) -> Result<Poll<()>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
pub trait Encode: Archive {
    type Encoder<'a>: Encoder<'a, Input = ()>
    where
        Self: 'a;

    fn begin_encode(&self) -> Result<Self::Encoder<'_>, ZebinError>;
}
