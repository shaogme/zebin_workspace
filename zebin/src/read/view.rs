use core::ops::Deref;
use alloc::string::ToString;

use crate::{
    core::schema::{LayoutDirectory, LayoutView},
    error::ZebinError,
    format::ArchiveHeader,
    traits::{Access, Archive},
    utils::num::u32_to_nonzero_usize,
};

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ArchiveView<'a, T: Archive>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    pub(crate) bytes: &'a [u8],
    pub(crate) header: ArchiveHeader,
    pub(crate) root: <T::Archived as Access<'a>>::View,
}

impl<'a, T: Archive> ArchiveView<'a, T>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> ArchiveHeader {
        self.header
    }

    pub fn root(&self) -> &<T::Archived as Access<'a>>::View {
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
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    type Target = <<T::Archived as Access<'a>>::View as core::ops::Deref>::Target;

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
