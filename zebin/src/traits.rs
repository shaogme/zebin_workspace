use core::num::NonZeroUsize;

use crate::core::schema::ObjectEncoding;
use crate::error::{AccessError, ArchiveError, ParseHeaderError, ValidateError};
use crate::validation::context::ValidationContext;
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

/// Re-export Serialize and SerializeState from write module.
pub use crate::write::state::{Serialize, SerializeState};

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
