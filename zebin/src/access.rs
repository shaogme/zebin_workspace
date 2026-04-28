use core::num::NonZeroUsize;
use core::ops::Deref;

use crate::{
    core::{
        schema::{LayoutDirectory, LayoutView},
        validator::Validator,
    },
    format::{ARCHIVE_HEADER_SIZE, ArchiveHeader},
    traits::{Archive, ArchivedDecode, ArchivedLayout, ArchivedValidate, ZebinError},
    utils::num::{u32_to_nonzero_usize, u32_to_usize},
};

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ArchiveView<'a, T: Archive>
where
    T::Archived: ArchivedDecode<'a>,
    <T::Archived as ArchivedDecode<'a>>::View: Deref,
{
    bytes: &'a [u8],
    header: ArchiveHeader,
    root: <T::Archived as ArchivedDecode<'a>>::View,
}

impl<'a, T: Archive> ArchiveView<'a, T>
where
    T::Archived: ArchivedDecode<'a>,
    <T::Archived as ArchivedDecode<'a>>::View: Deref,
{
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> ArchiveHeader {
        self.header
    }

    pub fn root(&self) -> &<T::Archived as ArchivedDecode<'a>>::View {
        &self.root
    }

    pub fn resolved_layout(
        &self,
        stable_schema_key: u32,
        schema_revision: u32,
    ) -> Result<ResolvedLayout<'a>, ZebinError> {
        ResolvedLayout::new(self.bytes, stable_schema_key, schema_revision)
    }
}

impl<'a, T: Archive> Deref for ArchiveView<'a, T>
where
    T::Archived: ArchivedDecode<'a>,
    <T::Archived as ArchivedDecode<'a>>::View: Deref,
{
    type Target = <<T::Archived as ArchivedDecode<'a>>::View as core::ops::Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.root.deref()
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

    pub fn field_offset(&self, field_id: u16) -> Option<u32> {
        self.layout.field_offset(field_id)
    }
}

/// Decode and validate the archived root object.
pub fn decode<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: ArchivedLayout + ArchivedValidate + ArchivedDecode<'a>,
    <T::Archived as ArchivedDecode<'a>>::View: Deref,
{
    check_archive(bytes)
}

/// Check the archive for safety and return a validated archive view.
fn check_archive<'a, T>(bytes: &'a [u8]) -> Result<ArchiveView<'a, T>, ZebinError>
where
    T: Archive,
    T::Archived: ArchivedLayout + ArchivedValidate + ArchivedDecode<'a>,
    <T::Archived as ArchivedDecode<'a>>::View: Deref,
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
        unsafe { <T::Archived as ArchivedDecode<'a>>::decode_view(root_ptr, &mut validator)? };
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
    T::Archived: ArchivedLayout + ArchivedValidate + ArchivedDecode<'a>,
    <T::Archived as ArchivedDecode<'a>>::View: Deref,
{
    decode::<T>(bytes).map(|_| ())
}
