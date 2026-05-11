use crate::{
    core::schema::{LayoutDirectory, LayoutView, ObjectEncoding},
    error::{ValidateError, ZebinError},
    format::ArchiveHeader,
    traits::{
        Access, Archive, ArchiveHeader as ArchiveHeaderTrait, Layout, Restore, SchemaAware,
        Validate,
    },
    utils::num::{u32_to_nonzero_usize, u32_to_usize},
    validation::validator::Validator,
};
use core::num::NonZeroUsize;
use core::ops::Deref;

fn validate_new_layout_header<H: ArchiveHeaderTrait>(header: &H) -> Result<(), ValidateError> {
    let layout_pos = u32_to_usize(header.layout_offset().get(), || {
        ValidateError::ValidationError {
            message: "Layout offset exceeds usize range",
            pos: H::LAYOUT_OFFSET_POS,
        }
    })?;
    let root_pos = u32_to_usize(header.root_offset().get(), || {
        ValidateError::ValidationError {
            message: "Root offset exceeds usize range",
            pos: H::ROOT_OFFSET_POS,
        }
    })?;
    if layout_pos < H::SIZE {
        return Err(ValidateError::ValidationError {
            message: "Layout overlaps archive header",
            pos: layout_pos,
        });
    }
    if layout_pos >= root_pos {
        return Err(ValidateError::ValidationError {
            message: "Layout must precede root",
            pos: layout_pos,
        });
    }
    Ok(())
}

/// Safe access layer output that keeps the validated byte slice alive.
///
/// ZebinReader acts as a Root View of the archive. It implements Deref to the root [View],
/// providing direct access to schema-aware fields and nested views.
pub struct ZebinReader<'a, T: Archive, H: ArchiveHeaderTrait = ArchiveHeader>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    view: View<'a, <T::Archived as Access<'a>>::View, H>,
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> ZebinReader<'a, T, H>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    pub fn bytes(&self) -> &'a [u8] {
        self.view.layout().bytes()
    }

    pub fn header(&self) -> &H {
        self.view.layout().header()
    }

    /// Returns the root object view.
    pub fn root(&self) -> &<T::Archived as Access<'a>>::View {
        &self.view.data
    }

    /// Wrap a schema-aware object into a view using this reader's archive context.
    pub fn view<S: SchemaAware>(&self, obj: &'a S) -> Result<View<'a, &'a S, H>, ZebinError> {
        self.view.view(obj)
    }

    /// Restore the original root object from this reader.
    pub fn restore(&self) -> Result<T, ZebinError>
    where
        View<'a, <T::Archived as Access<'a>>::View, H>: Restore<T>,
    {
        self.view.restore()
    }

    /// Create a reader for the archived root object.
    pub fn new(bytes: &'a [u8]) -> Result<Self, ZebinError>
    where
        T::Archived: Layout + Validate,
    {
        let header = H::parse(bytes)?;
        validate_new_layout_header(&header)?;
        let root_pos = u32_to_usize(header.root_offset().get(), || {
            ValidateError::ValidationError {
                message: "Root offset exceeds usize range",
                pos: H::ROOT_OFFSET_POS,
            }
        })?;

        if root_pos % <T::Archived as Layout>::ALIGNMENT.get() != 0 {
            return Err(ValidateError::AlignmentError {
                expected: <T::Archived as Layout>::ALIGNMENT,
                actual: unsafe {
                    NonZeroUsize::new_unchecked(root_pos % <T::Archived as Layout>::ALIGNMENT.get())
                },
                pos: root_pos,
            }
            .into());
        }

        let layout_dir = LayoutDirectory::new(
            bytes,
            u32_to_nonzero_usize(
                header.layout_offset().get(),
                || ValidateError::ValidationError {
                    message: "Layout offset exceeds usize range",
                    pos: H::LAYOUT_OFFSET_POS,
                },
                || ValidateError::ValidationError {
                    message: "Layout offset cannot be zero",
                    pos: H::LAYOUT_OFFSET_POS,
                },
            )?,
        )?;
        let layout_end = layout_dir.section_end();
        if layout_end > root_pos {
            return Err(ValidateError::ValidationError {
                message: "Layout section overlaps root",
                pos: layout_end,
            }
            .into());
        }
        let mut validator = Validator::<'a, H>::with_layouts(bytes, header, layout_dir);
        let root_ptr = unsafe { bytes.as_ptr().add(root_pos) };
        let (root_view, root_span) =
            unsafe { <T::Archived as Access<'a>>::access(root_ptr, &mut validator)? };
        let root_end = root_pos
            .checked_add(root_span)
            .ok_or(ValidateError::ValidationError {
                message: "Root range overflow",
                pos: root_pos,
            })?;
        if root_end > bytes.len() {
            return Err(ValidateError::ValidationError {
                message: "Root out of bounds",
                pos: root_pos,
            }
            .into());
        }

        // Automatically resolve root layout if it's schema-aware
        let layout = if <T::Archived as Layout>::ENCODING == ObjectEncoding::SchemaAware {
            // Safety: Schema-aware objects MUST start with 4-byte key and 4-byte revision
            let key = unsafe { *(root_ptr as *const u32) };
            let rev = unsafe { *(root_ptr as *const u32).add(1) };
            ResolvedLayout::from_parts(bytes, header, Some(layout_dir.lookup(key, rev)?))
        } else {
            ResolvedLayout::context_only(bytes, header)
        };

        Ok(Self {
            view: View::new_with_layout(root_view, layout),
        })
    }

    /// Decode and validate the archived root object directly into the original type T.
    pub fn decode(bytes: &'a [u8]) -> Result<T, ZebinError>
    where
        T::Archived: Layout + Validate,
        View<'a, <T::Archived as Access<'a>>::View, H>: Restore<T>,
    {
        Self::new(bytes)?.restore()
    }

    /// Validate an archive without exposing the archived view.
    pub fn validate(bytes: &'a [u8]) -> Result<(), ZebinError>
    where
        T::Archived: Layout + Validate,
    {
        Self::new(bytes).map(|_| ())
    }
}

impl<'a, T: Archive, H: ArchiveHeaderTrait> Deref for ZebinReader<'a, T, H>
where
    T::Archived: Access<'a>,
    <T::Archived as Access<'a>>::View: Deref,
{
    type Target = View<'a, <T::Archived as Access<'a>>::View, H>;

    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

/// Resolved layout handle for a specific schema key and revision.
#[derive(Clone, Copy)]
pub struct ResolvedLayout<'a, H: ArchiveHeaderTrait = ArchiveHeader> {
    bytes: &'a [u8],
    header: H,
    layout: Option<LayoutView<'a>>,
}

impl<'a, H: ArchiveHeaderTrait> ResolvedLayout<'a, H> {
    pub(crate) fn from_parts(bytes: &'a [u8], header: H, layout: Option<LayoutView<'a>>) -> Self {
        Self {
            bytes,
            header,
            layout,
        }
    }

    /// Create a layout that only provides archive context (no object fields).
    pub fn context_only(bytes: &'a [u8], header: H) -> Self {
        Self::from_parts(bytes, header, None)
    }

    pub fn new(
        bytes: &'a [u8],
        stable_schema_key: u32,
        schema_revision: u32,
    ) -> Result<Self, ZebinError> {
        let header = H::parse(bytes)?;
        validate_new_layout_header(&header)?;
        let layout_dir = LayoutDirectory::new(
            bytes,
            u32_to_nonzero_usize(
                header.layout_offset().get(),
                || ValidateError::ValidationError {
                    message: "Layout offset exceeds usize range",
                    pos: H::LAYOUT_OFFSET_POS,
                },
                || ValidateError::ValidationError {
                    message: "Layout offset cannot be zero",
                    pos: H::LAYOUT_OFFSET_POS,
                },
            )?,
        )?;
        let root_pos = u32_to_usize(header.root_offset().get(), || {
            ValidateError::ValidationError {
                message: "Root offset exceeds usize range",
                pos: H::ROOT_OFFSET_POS,
            }
        })?;
        let layout_end = layout_dir.section_end();
        if layout_end > root_pos {
            return Err(ValidateError::ValidationError {
                message: "Layout section overlaps root",
                pos: layout_end,
            }
            .into());
        }
        let layout = layout_dir.lookup(stable_schema_key, schema_revision)?;
        Ok(Self::from_parts(bytes, header, Some(layout)))
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn header(&self) -> &H {
        &self.header
    }

    pub fn layout(&self) -> Option<LayoutView<'a>> {
        self.layout
    }

    pub fn stable_schema_key(&self) -> u32 {
        self.layout.map(|l| l.stable_schema_key()).unwrap_or(0)
    }

    pub fn schema_revision(&self) -> u32 {
        self.layout.map(|l| l.schema_revision()).unwrap_or(0)
    }

    pub fn field_offset(&self, field_id: u16) -> Option<u32> {
        self.layout.and_then(|l| l.field_offset(field_id))
    }

    pub fn check_field(&self, field_id: u16, expected: u32) -> Result<(), ValidateError> {
        self.layout
            .ok_or(ValidateError::ValidationError {
                message: "Missing layout for field check",
                pos: 0,
            })?
            .check_field(field_id, expected)
    }

    /// Resolve a nested layout using the same archive header and byte source.
    pub fn resolve_nested(
        &self,
        stable_schema_key: u32,
        schema_revision: u32,
    ) -> Result<Self, ZebinError> {
        Self::new(self.bytes, stable_schema_key, schema_revision)
    }
}

/// Helper for resolving a nested layout from a context layout and an archived field.
pub fn get_nested_layout<'a, T: Layout, H: ArchiveHeaderTrait>(
    context: &ResolvedLayout<'a, H>,
    data: &T,
) -> Result<ResolvedLayout<'a, H>, ZebinError> {
    if T::ENCODING == crate::core::schema::ObjectEncoding::SchemaAware {
        // Safety: Schema-aware objects MUST start with 4-byte key and 4-byte revision.
        // We know T is SchemaAware because of the ENCODING check.
        let ptr = data as *const T as *const u32;
        let key = unsafe { *ptr };
        let rev = unsafe { *ptr.add(1) };
        context.resolve_nested(key, rev)
    } else {
        Ok(ResolvedLayout::context_only(
            context.bytes(),
            *context.header(),
        ))
    }
}

/// A view wrapper that binds an archived object with its resolved layout.
pub struct View<'a, T, H: ArchiveHeaderTrait = ArchiveHeader> {
    data: T,
    layout: ResolvedLayout<'a, H>,
}

impl<'a, T, H: ArchiveHeaderTrait> View<'a, T, H> {
    /// Create a new view from archived data and its layout.
    pub fn new_with_layout(data: T, layout: ResolvedLayout<'a, H>) -> Self {
        Self { data, layout }
    }

    /// Returns the raw archived data.
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Returns the resolved layout.
    pub fn layout(&self) -> &ResolvedLayout<'a, H> {
        &self.layout
    }

    /// Resolve a view for a nested schema-aware object using this view's context.
    pub fn view<U: SchemaAware>(&self, data: &'a U) -> Result<View<'a, &'a U, H>, ZebinError> {
        let layout = self
            .layout
            .resolve_nested(data.stable_schema_key(), data.schema_revision())?;
        Ok(View::new_with_layout(data, layout))
    }

    /// Restore the original object from this view.
    pub fn restore<U>(&self) -> Result<U, ZebinError>
    where
        Self: Restore<U>,
    {
        Restore::restore(self)
    }
}

impl<'a, T: Deref, H: ArchiveHeaderTrait> Deref for View<'a, T, H> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        self.data.deref()
    }
}
