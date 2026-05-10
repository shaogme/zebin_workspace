use core::num::NonZeroUsize;

use crate::core::schema::ObjectEncoding;
use crate::error::{AccessError, ArchiveError, ParseHeaderError, ValidateError};
use crate::validation::context::ValidationContext;
use crate::{LayoutField, ResolvedLayout, SchemaRevision, StableSchemaKey, ZebinError};
use core::num::NonZeroU32;

/// Archived-side binary layout contract.
pub trait Layout: Sized {
    /// The alignment requirement for the archived representation.
    const ALIGNMENT: NonZeroUsize;

    /// Object encoding family used in schema metadata and archive flags.
    const ENCODING: ObjectEncoding = ObjectEncoding::Fixed;

    /// Optional byte width used when the archived form is not a plain fixed-size overlay.
    fn size_hint(&self) -> usize {
        core::mem::size_of::<Self>()
    }

    /// Write a deterministic byte representation of an archived value.
    fn write_archived_bytes(archived: &Self, out: &mut [u8]);
}

/// Archive format header contract.
pub trait ArchiveHeader: Layout + Clone + Copy {
    /// Fixed-size byte array type for the header.
    type Bytes: AsRef<[u8]> + Copy + Send + Sync;

    /// Magic bytes for the archive format.
    const MAGIC: [u8; 2];
    /// Format version.
    const VERSION: u8;
    /// Fixed size of the header in bytes.
    const SIZE: usize;

    /// Parse header from bytes.
    fn parse(bytes: &[u8]) -> Result<Self, ParseHeaderError>;

    /// Encode header into its fixed-size byte representation.
    fn encode(&self) -> Self::Bytes;

    /// Create header instance from metadata.
    fn create(flags: u8, layout_offset: NonZeroU32, root_offset: NonZeroU32) -> Self;

    /// Get the layout section offset.
    fn layout_offset(&self) -> NonZeroU32;

    /// Get the root object offset.
    fn root_offset(&self) -> NonZeroU32;
}

/// Archived-side validation contract.
pub trait Validate {
    /// Validate an archived value in-place.
    ///
    /// # Safety
    /// The pointer must point to a valid memory location that can be read.
    unsafe fn validate<H, C>(_ptr: *const Self, _context: &mut C) -> Result<(), ValidateError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized;
}

/// Archived-side decode contract for borrowing a validated view from bytes.
pub trait Access<'a>: Sized {
    type View: 'a;

    /// Decode a borrowed view from a validated archived pointer.
    ///
    /// The returned span is the number of bytes consumed by this archived value.
    ///
    /// # Safety
    /// `ptr` must point to the first byte of a readable archived value at `context`'s current archive.
    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized;
}

/// Object model layer: type-level archive/serialize/validate contracts.
pub trait Archive {
    /// The archived version of this type.
    type Archived: Layout + Validate;
    /// The resolver used to construct the archived version.
    type Resolver;

    /// Resolve the archived version using the given resolver.
    fn resolve(
        &self,
        archive_pos: usize,
        resolver: Self::Resolver,
    ) -> Result<Self::Archived, ArchiveError>;
}

/// Contract for providing a static default archived value.
pub trait ArchivedDefault {
    /// Returns a reference to a static archived default value.
    fn archived_default() -> &'static Self;
}

/// Contract for archived representations that carry their own schema metadata.
pub trait SchemaAware {
    /// Returns the stable schema key for this archived object.
    fn stable_schema_key(&self) -> u32;
    /// Returns the schema revision for this archived object.
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

/// Contract for types that can be restored to their original form from an archived representation.
pub trait Restore<T> {
    /// Restores the original value from the archived representation.
    fn restore(&self) -> Result<T, ZebinError>;
}

/// Contract for schema-aware archived types that require a layout for restoration.
pub trait RestoreFromView<'a, T, H: ArchiveHeader> {
    /// Restores the original value using the provided resolved layout.
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<T, ZebinError>;
}

impl<'a, T, U, H: ArchiveHeader> RestoreFromView<'a, U, H> for &'a T
where
    T: RestoreFromView<'a, U, H> + Layout,
{
    fn restore_from_view(&self, layout: &ResolvedLayout<'a, H>) -> Result<U, ZebinError> {
        let item_layout = crate::read::get_nested_layout(layout, *self)?;
        (*self).restore_from_view(&item_layout)
    }
}

/// Byte-stream sink used by archive state machines.
pub trait ByteSink {
    fn pos(&self) -> usize;

    /// Write as many bytes as possible and return the amount consumed.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, ZebinError>;

    /// Write as many alignment bytes as possible and return the amount consumed.
    fn align(&mut self, alignment: NonZeroUsize) -> Result<usize, ZebinError>;

    /// Advance the position without writing actual data.
    fn skip(&mut self, len: usize) -> Result<usize, ZebinError>;
}

/// Layout registration sink used by archive state machines.
pub trait LayoutSink<'a> {
    /// Register a layout descriptor for the current object.
    fn register_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
        encoding: ObjectEncoding,
        layout: &'a [LayoutField],
    ) -> Result<(), ZebinError>;
}

/// Trait for resumable archive construction states.
pub trait SerializeState<'a> {
    type Resolver;

    fn poll<E: ByteSink + LayoutSink<'a> + ?Sized>(
        &mut self,
        encoder: &mut E,
    ) -> Result<core::task::Poll<Self::Resolver>, ZebinError>;
}

/// Trait for types that can create resumable archive states.
pub trait Serialize: Archive {
    type State<'a>: SerializeState<'a, Resolver = Self::Resolver>
    where
        Self: 'a;

    fn begin_serialize(&self) -> Result<Self::State<'_>, ZebinError>;
}

impl<T: Layout, const N: usize> Layout for [T; N] {
    const ALIGNMENT: NonZeroUsize = T::ALIGNMENT;

    fn write_archived_bytes(archived: &Self, out: &mut [u8]) {
        crate::utils::byteops::fill(out, 0);
        let elem_size = core::mem::size_of::<T>();
        if elem_size == 0 {
            return;
        }
        for (index, item) in archived.iter().enumerate() {
            let start = index * elem_size;
            let end = start + elem_size;
            T::write_archived_bytes(item, &mut out[start..end]);
        }
    }
}

impl<T: Layout + Validate, const N: usize> Validate for [T; N] {
    unsafe fn validate<H, C>(ptr: *const Self, context: &mut C) -> Result<(), ValidateError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let mut guard = context.guard()?;
        guard.check_alignment(ptr as *const u8, Self::ALIGNMENT)?;
        guard.check_range(ptr as *const u8, core::mem::size_of::<Self>())?;
        let data_ptr = ptr as *const T;
        let elem_size = core::mem::size_of::<T>();

        for index in 0..N {
            let element_ptr = if elem_size == 0 {
                data_ptr
            } else {
                unsafe { data_ptr.add(index) }
            };
            {
                let mut _path_guard = guard.push_index(index);
                unsafe {
                    T::validate::<H, _>(element_ptr, &mut *_path_guard)?;
                }
            }
        }

        Ok(())
    }
}

impl<'a, T, const N: usize> Access<'a> for [T; N]
where
    T: Layout + Validate + 'a,
{
    type View = &'a Self;

    unsafe fn access<H, C>(
        ptr: *const u8,
        context: &mut C,
    ) -> Result<(Self::View, usize), AccessError>
    where
        H: ArchiveHeader,
        C: ValidationContext<H> + ?Sized,
    {
        let typed_ptr = ptr as *const Self;
        unsafe {
            <Self as Validate>::validate::<H, C>(typed_ptr, context)?;
        }
        Ok((unsafe { &*typed_ptr }, core::mem::size_of::<Self>()))
    }
}

pub struct OptRestorer<'a, A, H: ArchiveHeader> {
    pub data: Option<&'a A>,
    pub layout: &'a crate::read::ResolvedLayout<'a, H>,
    pub error_msg: &'static str,
}

pub trait OptRestorerFallback<'a, T, H: ArchiveHeader> {
    fn restore(self) -> Result<T, ZebinError>;
}

impl<'a, A, T, H: ArchiveHeader> OptRestorerFallback<'a, T, H> for &OptRestorer<'a, A, H>
where
    A: RestoreFromView<'a, T, H> + Layout,
{
    fn restore(self) -> Result<T, ZebinError> {
        match self.data {
            Some(archived) => {
                let nested = crate::read::get_nested_layout(self.layout, archived)?;
                archived.restore_from_view(&nested)
            }
            None => Err(ZebinError::DeserializeError {
                message: self.error_msg,
            }),
        }
    }
}

pub trait OptRestorerOption<'a, T, H: ArchiveHeader> {
    fn restore(self) -> Result<Option<T>, ZebinError>;
}
