use crate::io::CursorMut;
use core::{num::NonZeroUsize, task::Poll};

#[cfg(feature = "alloc")]
use crate::archive_impl::skip_block_index;
use crate::{error::ParseHeaderError, prelude::*};

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Fixed-width archived overlay contract.
///
/// Only plain fixed-size overlays implement this trait. Variable-width archive
/// forms are deserialized through [`Access`] instead of pretending to have a stable
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

    fn serialize(&self) -> Self::Bytes;

    fn create(flags: u8) -> Self;
}

/// Static archived layout metadata shared by read and write paths.
pub trait ArchivedLayout {
    const FIXED_SIZE: Option<usize> = None;
    const ALIGNMENT: NonZeroUsize = NonZeroUsize::new(1).unwrap();
    const OBJECT_ENCODING: ObjectEncoding = ObjectEncoding::Fixed;
    const FIELD_ENCODING: FieldEncoding = FieldEncoding::Fixed;
}

/// Read-side deserialize contract for consuming a value from a sequential cursor.
pub trait Access: ArchivedLayout + Sized {
    type View<'a>
    where
        Self: 'a;
    #[cfg(feature = "alloc")]
    type AccessStrategy: SequenceAccessStrategy<Self>;

    fn access<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<Self::View<'a>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
        Self: 'a;

    fn validate<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized;
}

/// Strategy for decoding and validating a sequence of elements.
#[cfg(feature = "alloc")]
pub trait SequenceAccessStrategy<T: Access> {
    fn access_sequence<'a, C, Cr>(
        cursor: &mut Cr,
        context: &mut C,
    ) -> Result<Vec<T::View<'a>>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized;

    fn validate_sequence<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized;
}

/// Strategy for fixed-size sequence elements (with alignment).
#[cfg(feature = "alloc")]
pub struct FixedSequenceStrategy;

#[cfg(feature = "alloc")]
impl<T: Access> SequenceAccessStrategy<T> for FixedSequenceStrategy {
    fn access_sequence<'a, C, Cr>(
        cursor: &mut Cr,
        context: &mut C,
    ) -> Result<Vec<T::View<'a>>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        let mut items = Vec::new();
        let mut index = 0;
        loop {
            let marker = cursor.read_u8(context)?;
            if marker == 0 {
                break;
            } else if marker != 1 {
                return Err(context.validation_error("Invalid sequence marker", cursor.pos() - 1));
            }
            cursor.align(T::ALIGNMENT, context)?;
            let mut guard = context.push_index(index);
            items.push(T::access(cursor, &mut *guard)?);
            index += 1;
        }
        // Skip trailing block index if present.
        skip_block_index(cursor, context)?;
        Ok(items)
    }

    fn validate_sequence<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        let mut index = 0;
        loop {
            let marker = cursor.read_u8(context)?;
            if marker == 0 {
                break;
            } else if marker != 1 {
                return Err(context.validation_error("Invalid sequence marker", cursor.pos() - 1));
            }
            cursor.align(T::ALIGNMENT, context)?;
            let mut guard = context.push_index(index);
            T::validate(cursor, &mut *guard)?;
            index += 1;
        }
        // Skip trailing block index if present.
        skip_block_index(cursor, context)?;
        Ok(())
    }
}

/// Strategy for forward self-describing variable-length elements.
#[cfg(feature = "alloc")]
pub struct ForwardSequenceStrategy;

#[cfg(feature = "alloc")]
impl<T: Access> SequenceAccessStrategy<T> for ForwardSequenceStrategy {
    fn access_sequence<'a, C, Cr>(
        cursor: &mut Cr,
        context: &mut C,
    ) -> Result<Vec<T::View<'a>>, AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        let mut items = Vec::new();
        let mut index = 0;
        loop {
            let marker = cursor.read_u8(context)?;
            if marker == 0 {
                break;
            } else if marker != 1 {
                return Err(context.validation_error("Invalid sequence marker", cursor.pos() - 1));
            }
            let mut guard = context.push_index(index);
            items.push(T::access(cursor, &mut *guard)?);
            index += 1;
        }
        // Skip trailing block index if present.
        skip_block_index(cursor, context)?;
        Ok(items)
    }

    fn validate_sequence<'a, C, Cr>(cursor: &mut Cr, context: &mut C) -> Result<(), AccessError>
    where
        C: ValidationContext + ?Sized,
        Cr: Cursor<'a> + ?Sized,
    {
        let mut index = 0;
        loop {
            let marker = cursor.read_u8(context)?;
            if marker == 0 {
                break;
            } else if marker != 1 {
                return Err(context.validation_error("Invalid sequence marker", cursor.pos() - 1));
            }
            let mut guard = context.push_index(index);
            T::validate(cursor, &mut *guard)?;
            index += 1;
        }
        // Skip trailing block index if present.
        skip_block_index(cursor, context)?;
        Ok(())
    }
}

/// Object model layer: type-level archive and deserialize contracts.
pub trait Archive {
    type Archived;
    const ALLOW_MISSING: bool = false;
}

/// Contract for providing a static default archived view value.
pub trait ArchivedDefault {
    fn archived_default() -> &'static Self;
}

/// Contract for schema-aware deserialized views.
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

/// Contract for deserialized views that can deserialize the source type.
pub trait Deserialize<T> {
    fn deserialize(&self) -> Result<T, ZebinError>;

    fn deserialize_missing() -> Result<T, ZebinError> {
        Err(ZebinError::DeserializeError {
            message: "Missing required field",
        })
    }
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

/// Unified serializer protocol, supporting one-off or incremental step-by-step input.
///
/// The trait carries no lifetime. Serializers that work over borrowed inputs (DST
/// adapters such as `[T]` / `str` / shared-pointer wrappers) embed any required
/// lifetime in their concrete type and feed it through `Serialize::Input<'a>`.
pub trait Serializer {
    /// The type of input items received by the serializer.
    ///
    /// For owned serializers this is the value type itself. For DST adapters this
    /// is a borrowed reference (e.g. `&'a [T]`).
    type Input;

    /// Attempts to input a data item into the serializer and serialize it into the underlying `CursorMut`.
    ///
    /// # Return Value
    /// - `Ok(Poll::Ready(()))`: The current input item has been fully serialized and written.
    /// - `Ok(Poll::Pending)`: The `CursorMut` is full. The current input item is pending, and the caller should flush/provide a new CursorMut and call `poll_pending`.
    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError>;

    /// Advances and flushes any state/data previously accumulated inside the serializer due to insufficient `CursorMut` space.
    ///
    /// Regardless of whether it is a one-off or step-by-step input, this method can be called to advance the remaining encoding progress until it returns `Poll::Ready(())`.
    fn poll_pending(&mut self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError>;

    /// Finishes the encoding process, writing any necessary alignments, paddings, or trailing metadata.
    fn finish(self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
///
/// `Input<'a>` defaults conceptually to `Self` for owned/sized types and is
/// overridden to `&'a Self` for DST adapters (`[T]`, `str`) and shared-pointer
/// wrappers (`Rc<T>`, `Arc<T>`, `Cow<'_, T>`) where moving by value is impossible
/// or would defeat the purpose of the wrapper.
pub trait Serialize: Archive {
    type Input<'a>
    where
        Self: 'a;

    type Serializer<'a>: Serializer<Input = Self::Input<'a>>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a;
}

/// Serializer for `&T` references.
///
/// RefSerializer takes a borrow of a reference `&'b T`, clones the inner `T` (which is `Clone`),
/// and feeds the cloned owned value into `T::Serializer`.
pub struct RefSerializer<'a, 'b, T>
where
    T: Serialize + Archive + 'a,
{
    inner: <T as Serialize>::Serializer<'a>,
    _phantom: core::marker::PhantomData<&'b T>,
}

impl<'a, 'b, T> RefSerializer<'a, 'b, T>
where
    T: Serialize + Archive + 'a,
{
    pub fn new() -> Self {
        Self {
            inner: T::serializer(),
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<'a, 'b, T> Default for RefSerializer<'a, 'b, T>
where
    T: Serialize + Archive + 'a,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, 'b, T> Serializer for RefSerializer<'a, 'b, T>
where
    T: Serialize<Input<'a> = T> + Archive + Clone + 'a,
{
    type Input = &'b T;

    fn input(
        &mut self,
        item: Self::Input,
        sink: &mut CursorMut<'_>,
    ) -> Result<Poll<()>, ZebinError> {
        let value: T = (*item).clone();
        self.inner.input(value, sink)
    }

    fn poll_pending(&mut self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.poll_pending(sink)
    }

    fn finish(self, sink: &mut CursorMut<'_>) -> Result<Poll<()>, ZebinError> {
        self.inner.finish(sink)
    }
}

impl<T: Archive + ?Sized> Archive for &T {
    type Archived = T::Archived;
}

impl<'b, T> Serialize for &'b T
where
    T: Serialize + Archive + Clone,
    for<'a> T: Serialize<Input<'a> = T> + 'a,
{
    type Input<'a>
        = &'b T
    where
        Self: 'a;
    type Serializer<'a>
        = RefSerializer<'a, 'b, T>
    where
        Self: 'a;

    fn serializer<'a>() -> Self::Serializer<'a>
    where
        Self: 'a,
    {
        RefSerializer::new()
    }
}

/// Pre-pass measurement contract.
///
/// Used by:
/// - schema-aware records that must know each field's serialized length before
///   writing the field table;
///
/// Implementations walk the value by reference and never consume it. They must
/// produce a length that exactly matches what the corresponding serializer will
/// write to a `StorageMut`.
pub trait MeasureBody {
    fn measure_body(&self) -> Result<usize, ZebinError>;
}

impl<T: MeasureBody + ?Sized> MeasureBody for &T {
    fn measure_body(&self) -> Result<usize, ZebinError> {
        (**self).measure_body()
    }
}

/// Helper contract for resolving fields, supporting default error for missing required fields
/// and None fallback for optional fields.
pub trait ArchivedField<'a>: Sized + 'a {
    #[inline]
    fn resolve_field(view: Option<&Self>, field_id: u16, pos: usize) -> Result<&Self, ZebinError> {
        view.ok_or(ZebinError::Access(AccessError::MissingField {
            field_id,
            pos,
        }))
    }
}

impl<'a, T: FixedLayout + 'a> ArchivedField<'a> for T {}
