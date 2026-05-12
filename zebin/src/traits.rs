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
        C: ValidationContext + ?Sized,
    {
        let _ = Self::decode(cursor, context)?;
        Ok(())
    }
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
    fn stable_schema_key(&self) -> u32;
    fn schema_revision(&self) -> u32;
}

impl<T: SchemaAware + ?Sized> SchemaAware for &T {
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

/// Byte-stream sink used by archive state machines.
///
/// Implementations may perform short writes. `write`, `align`, and `skip`
/// return the number of bytes actually accepted; callers must preserve their
/// own cursor state and return `Poll::Pending` when more output space is
/// needed.
pub trait ByteSink {
    fn pos(&self) -> usize;

    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError>;

    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError>;

    fn skip(&mut self, len: usize) -> Result<usize, ZebinError>;
}

/// Trait for resumable sequential archive construction states.
///
/// A state should make as much progress as the provided [`ByteSink`] allows.
/// When a sink accepts only part of the requested bytes, the state must retain
/// enough progress to resume the next call without rewriting bytes.
pub trait SerializeState<'a> {
    fn poll<E: ByteSink + ?Sized>(&mut self, encoder: &mut E) -> Result<Poll<()>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
pub trait Serialize: Archive {
    type State<'a>: SerializeState<'a>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError>;
}
