pub mod view;

use core::num::NonZeroUsize;
use core::ops::Deref;
use alloc::string::ToString;

use crate::{
    core::schema::LayoutDirectory,
    error::ZebinError,
    format::{ARCHIVE_HEADER_SIZE, ArchiveHeader},
    read::view::ArchiveView,
    traits::{Access, Archive, Layout, Validate},
    utils::num::u32_to_usize,
    validation::validator::Validator,
};

/// Decode and validate the archived root object.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    check_archive(bytes)
}

/// Check the archive for safety and return a validated archive view.
fn check_archive<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    let header = ArchiveHeader::parse(bytes)?;
    let root_pos = u32_to_usize(header.root_offset.get(), || ZebinError::ValidationError {
        message: "Root offset exceeds usize range".to_string(),
        pos: 8,
    })?;
    if root_pos < ARCHIVE_HEADER_SIZE {
        return Err(ZebinError::ValidationError {
            message: "Root overlaps archive header".to_string(),
            pos: root_pos,
        });
    }
    if root_pos % <T::Archived as Layout>::ALIGNMENT.get() != 0 {
        return Err(ZebinError::AlignmentError {
            expected: <T::Archived as Layout>::ALIGNMENT,
            actual: unsafe {
                NonZeroUsize::new_unchecked(root_pos % <T::Archived as Layout>::ALIGNMENT.get())
            },
            pos: root_pos,
        });
    }

    let layout_offset = u32_to_usize(header.layout_offset.get(), || ZebinError::ValidationError {
        message: "Layout offset exceeds usize range".to_string(),
        pos: 4,
    })?;

    use crate::utils::num::u32_to_nonzero_usize;
    let layout_dir = LayoutDirectory::new(
        bytes,
        u32_to_nonzero_usize(
            header.layout_offset.get(),
            || ZebinError::ValidationError {
                message: "Layout offset exceeds usize range".to_string(),
                pos: 4,
            },
            || ZebinError::ValidationError {
                message: "Layout offset cannot be zero".to_string(),
                pos: 4,
            },
        )?,
    )?;
    let mut validator = Validator::with_layouts(bytes, layout_dir);
    let root_ptr = unsafe { bytes.as_ptr().add(root_pos) };
    let (root_view, root_span) =
        unsafe { <T::Archived as Access<'a>>::access(root_ptr, &mut validator)? };
    let root_end = root_pos
        .checked_add(root_span)
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Root range overflow".to_string(),
            pos: root_pos,
        })?;
    if root_end > bytes.len() {
        return Err(ZebinError::ValidationError {
            message: "Root out of bounds".to_string(),
            pos: root_pos,
        });
    }
    if layout_offset < root_end {
        return Err(ZebinError::ValidationError {
            message: "Layout section overlaps root".to_string(),
            pos: layout_offset,
        });
    }

    Ok(ArchiveView {
        bytes,
        header,
        root: root_view,
    })
}

/// Validate an archive without exposing the archived view.
pub fn validate<'a, T>(bytes: &'a [u8]) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: 'a,
    T::Archived: Layout + Validate + Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    decode::<T>(bytes).map(|_| ())
}
