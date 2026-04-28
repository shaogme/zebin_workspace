use core::num::NonZeroUsize;
use core::ops::Deref;

use crate::{
    core::{
        schema::{LayoutDirectory, LayoutView},
        validator::Validator,
    },
    format::{ARCHIVE_HEADER_SIZE, ArchiveHeader},
    num::{u32_to_nonzero_usize, u32_to_usize},
    traits::{Archive, ArchivedLayout, ArchivedValidate, ZebinError},
};

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ArchiveView<'a, T: Archive> {
    bytes: &'a [u8],
    header: ArchiveHeader,
    root: &'a T::Archived,
}

impl<'a, T: Archive> ArchiveView<'a, T> {
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> ArchiveHeader {
        self.header
    }

    pub fn root(&self) -> &'a T::Archived {
        self.root
    }

    pub fn resolved_layout(
        &self,
        stable_schema_key: u32,
        schema_revision: u32,
    ) -> Result<ResolvedLayout<'a>, ZebinError> {
        ResolvedLayout::new(self.bytes, stable_schema_key, schema_revision)
    }
}

impl<'a, T: Archive> Deref for ArchiveView<'a, T> {
    type Target = T::Archived;

    fn deref(&self) -> &Self::Target {
        self.root
    }
}

/// Resolved layout handle for a specific schema key and revision.
#[derive(Clone, Copy)]
pub struct ResolvedLayout<'a> {
    bytes: &'a [u8],
    header: ArchiveHeader,
    layout: LayoutView<'a>,
}

impl<'a> ResolvedLayout<'a> {
    pub(crate) fn from_parts(
        bytes: &'a [u8],
        header: ArchiveHeader,
        layout: LayoutView<'a>,
    ) -> Self {
        Self {
            bytes,
            header,
            layout,
        }
    }

    pub fn new(
        bytes: &'a [u8],
        stable_schema_key: u32,
        schema_revision: u32,
    ) -> Result<Self, ZebinError> {
        let header = ArchiveHeader::parse(bytes)?;
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
        let layout = layout_dir.lookup(stable_schema_key, schema_revision)?;
        Ok(Self::from_parts(bytes, header, layout))
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> ArchiveHeader {
        self.header
    }

    pub fn layout(&self) -> LayoutView<'a> {
        self.layout
    }

    pub fn stable_schema_key(&self) -> u32 {
        self.layout.stable_schema_key()
    }

    pub fn schema_revision(&self) -> u32 {
        self.layout.schema_revision()
    }

    pub fn field_offset(&self, field_id: u16) -> Option<u16> {
        self.layout.field_offset(field_id)
    }
}

/// Decode and validate the archived root object.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: ArchivedLayout + ArchivedValidate,
{
    check_archive(bytes)
}

/// Check the archive for safety and return a validated archive view.
fn check_archive<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: ArchivedLayout + ArchivedValidate,
{
    let header = ArchiveHeader::parse(bytes)?;
    let root_pos = u32_to_usize(header.root_offset.get(), || ZebinError::ValidationError {
        message: "Root offset exceeds usize range".to_string(),
        pos: 8,
    })?;
    let root_end = root_pos
        .checked_add(core::mem::size_of::<T::Archived>())
        .ok_or_else(|| ZebinError::ValidationError {
            message: "Root range overflow".to_string(),
            pos: root_pos,
        })?;
    if root_pos < ARCHIVE_HEADER_SIZE {
        return Err(ZebinError::ValidationError {
            message: "Root overlaps archive header".to_string(),
            pos: root_pos,
        });
    }
    if root_end > bytes.len() {
        return Err(ZebinError::ValidationError {
            message: "Root out of bounds".to_string(),
            pos: root_pos,
        });
    }
    if root_pos % <T::Archived as ArchivedLayout>::ALIGNMENT.get() != 0 {
        return Err(ZebinError::AlignmentError {
            expected: <T::Archived as ArchivedLayout>::ALIGNMENT,
            actual: unsafe {
                NonZeroUsize::new_unchecked(
                    root_pos % <T::Archived as ArchivedLayout>::ALIGNMENT.get(),
                )
            },
            pos: root_pos,
        });
    }

    let layout_offset = u32_to_usize(header.layout_offset.get(), || ZebinError::ValidationError {
        message: "Layout offset exceeds usize range".to_string(),
        pos: 4,
    })?;
    if layout_offset < root_end {
        return Err(ZebinError::ValidationError {
            message: "Layout section overlaps root".to_string(),
            pos: layout_offset,
        });
    }
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
    let root_ptr = unsafe { bytes.as_ptr().add(root_pos) as *const T::Archived };

    unsafe {
        <T::Archived as ArchivedValidate>::validate(root_ptr, &mut validator)?;
    }

    Ok(ArchiveView {
        bytes,
        header,
        root: unsafe { &*root_ptr },
    })
}

/// Validate an archive without exposing the archived view.
pub fn validate<'a, T>(bytes: &'a [u8]) -> Result<(), ZebinError>
where
    T: Archive,
    T::Archived: 'a,
    T::Archived: ArchivedLayout + ArchivedValidate,
{
    decode::<T>(bytes).map(|_| ())
}
