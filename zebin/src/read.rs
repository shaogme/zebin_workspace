use crate::{
    core::schema::{LayoutDirectory, LayoutView},
    error::{ValidateError, ZebinError},
    format::ArchiveHeader,
    traits::{Access, Archive, ArchiveHeader as ArchiveHeaderTrait, Layout, Validate},
    utils::num::{u32_to_nonzero_usize, u32_to_usize},
    validation::validator::Validator,
};
use core::num::NonZeroUsize;
use core::ops::Deref;

/// Safe access layer output that keeps the validated byte slice alive.
pub struct ZebinReader<'a, T: Archive, H: ArchiveHeaderTrait = ArchiveHeader>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    pub(crate) bytes: &'a [u8],
    pub(crate) header: H,
    pub(crate) root: <T::Archived as Access<'a>>::View,
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> ZebinReader<'a, T, H>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> &H {
        &self.header
    }

    pub fn root(&self) -> &<T::Archived as Access<'a>>::View {
        &self.root
    }

    pub fn resolved_layout(
        &self,
        stable_schema_key: u32,
        schema_revision: u32,
    ) -> Result<ResolvedLayout<'a, H>, ZebinError> {
        ResolvedLayout::new(self.bytes, stable_schema_key, schema_revision)
    }

    /// Decode and validate the archived root object.
    pub fn decode(bytes: &'a [u8]) -> Result<Self, ZebinError>
    where
        T::Archived: Layout + Validate,
    {
        let header = H::parse(bytes)?;
        let root_pos = u32_to_usize(header.root_offset().get(), || ValidateError::ValidationError {
            message: "Root offset exceeds usize range",
            pos: 8,
            path: Default::default(),
        })?;
        if root_pos < H::SIZE {
            return Err(ValidateError::ValidationError {
                message: "Root overlaps archive header",
                pos: root_pos,
                path: Default::default(),
            }
            .into());
        }
        if root_pos % <T::Archived as Layout>::ALIGNMENT.get() != 0 {
            return Err(ValidateError::AlignmentError {
                expected: <T::Archived as Layout>::ALIGNMENT,
                actual: unsafe {
                    NonZeroUsize::new_unchecked(root_pos % <T::Archived as Layout>::ALIGNMENT.get())
                },
                pos: root_pos,
                path: Default::default(),
            }
            .into());
        }

        let _layout_offset = u32_to_usize(header.layout_offset().get(), || {
            ValidateError::ValidationError {
                message: "Layout offset exceeds usize range",
                pos: 4,
                path: Default::default(),
            }
        })?;

        let layout_dir = LayoutDirectory::new(
            bytes,
            u32_to_nonzero_usize(
                header.layout_offset().get(),
                || ValidateError::ValidationError {
                    message: "Layout offset exceeds usize range",
                    pos: 4,
                    path: Default::default(),
                },
                || ValidateError::ValidationError {
                    message: "Layout offset cannot be zero",
                    pos: 4,
                    path: Default::default(),
                },
            )?,
        )?;
        let mut validator = Validator::<'a, H>::with_layouts(bytes, header, layout_dir);
        let root_ptr = unsafe { bytes.as_ptr().add(root_pos) };
        let (root_view, root_span) =
            unsafe { <T::Archived as Access<'a>>::access(root_ptr, &mut validator)? };
        let root_end =
            root_pos
                .checked_add(root_span)
                .ok_or_else(|| ValidateError::ValidationError {
                    message: "Root range overflow",
                    pos: root_pos,
                    path: Default::default(),
                })?;
        if root_end > bytes.len() {
            return Err(ValidateError::ValidationError {
                message: "Root out of bounds",
                pos: root_pos,
                path: Default::default(),
            }
            .into());
        }
        let layout_offset = header.layout_offset().get() as usize;
        if layout_offset < root_end {
            return Err(ValidateError::ValidationError {
                message: "Layout section overlaps root",
                pos: layout_offset,
                path: Default::default(),
            }
            .into());
        }

        Ok(Self {
            bytes,
            header,
            root: root_view,
        })
    }

    /// Validate an archive without exposing the archived view.
    pub fn validate(bytes: &'a [u8]) -> Result<(), ZebinError>
    where
        T::Archived: Layout + Validate,
    {
        Self::decode(bytes).map(|_| ())
    }
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> Deref for ZebinReader<'a, T, H>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    type Target = <<T::Archived as Access<'a>>::View as Deref>::Target;

    fn deref(&self) -> &Self::Target {
        self.root.deref()
    }
}

/// Resolved layout handle for a specific schema key and revision.
#[derive(Clone, Copy)]
pub struct ResolvedLayout<'a, H: ArchiveHeaderTrait = ArchiveHeader> {
    bytes: &'a [u8],
    header: H,
    layout: LayoutView<'a>,
}

impl<'a, H: ArchiveHeaderTrait> ResolvedLayout<'a, H> {
    pub(crate) fn from_parts(bytes: &'a [u8], header: H, layout: LayoutView<'a>) -> Self {
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
        let header = H::parse(bytes)?;
        let layout_dir = LayoutDirectory::new(
            bytes,
            u32_to_nonzero_usize(
                header.layout_offset().get(),
                || ValidateError::ValidationError {
                    message: "Layout offset exceeds usize range",
                    pos: 4,
                    path: Default::default(),
                },
                || ValidateError::ValidationError {
                    message: "Layout offset cannot be zero",
                    pos: 4,
                    path: Default::default(),
                },
            )?,
        )?;
        let layout = layout_dir.lookup(stable_schema_key, schema_revision)?;
        Ok(Self::from_parts(bytes, header, layout))
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> &H {
        &self.header
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
