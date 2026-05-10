use crate::{
    core::schema::{SchemaRevision, StableSchemaKey},
    error::{ValidateError, ValidationPathSegment},
    format::ArchiveHeader,
    read::ResolvedLayout,
    traits::ArchiveHeader as ArchiveHeaderTrait,
};
use core::{marker::PhantomData, num::NonZeroUsize};

/// Validation context used by archived representations.
pub trait ValidationContext<H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    fn push_depth(&mut self) -> Result<(), ValidateError>;

    fn pop_depth(&mut self);

    fn push_path(&mut self, segment: ValidationPathSegment);

    fn pop_path(&mut self);

    fn record_error_path(&mut self);

    fn guard(&mut self) -> Result<ArchivedDepthGuard<'_, Self, H>, ValidateError> {
        ArchivedDepthGuard::new(self)
    }

    fn push_field(&mut self, name: &'static str) -> PathGuard<'_, Self, H> {
        PathGuard::new(self, ValidationPathSegment::Field(name))
    }

    fn push_index(&mut self, index: usize) -> PathGuard<'_, Self, H> {
        PathGuard::new(self, ValidationPathSegment::Index(index))
    }

    fn push_variant(&mut self, name: &'static str) -> PathGuard<'_, Self, H> {
        PathGuard::new(self, ValidationPathSegment::Variant(name))
    }

    fn check_range(&mut self, ptr: *const u8, size: usize) -> Result<(), ValidateError>;

    fn check_alignment(
        &mut self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ValidateError>;

    fn validation_error(&mut self, message: &'static str, pos: usize) -> ValidateError {
        self.record_error_path();
        ValidateError::ValidationError { message, pos }
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'_, H>, ValidateError>;
}

/// RAII guard that restores validation depth when dropped.
pub struct ArchivedDepthGuard<'a, C: ValidationContext<H> + ?Sized, H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    context: &'a mut C,
    _phantom: PhantomData<H>,
}

impl<'a, C, H> ArchivedDepthGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    pub fn new(context: &'a mut C) -> Result<Self, ValidateError> {
        context.push_depth()?;
        Ok(Self {
            context,
            _phantom: PhantomData,
        })
    }

    pub fn check_range(&mut self, ptr: *const u8, size: usize) -> Result<(), ValidateError> {
        self.context.check_range(ptr, size)
    }

    pub fn check_alignment(
        &mut self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ValidateError> {
        self.context.check_alignment(ptr, alignment)
    }
}

impl<'a, C, H> core::ops::Deref for ArchivedDepthGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<'a, C, H> core::ops::DerefMut for ArchivedDepthGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

impl<'a, C, H> Drop for ArchivedDepthGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    fn drop(&mut self) {
        self.context.pop_depth();
    }
}

/// RAII guard that restores validation path when dropped.
pub struct PathGuard<'a, C: ValidationContext<H> + ?Sized, H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    context: &'a mut C,
    _phantom: PhantomData<H>,
}

impl<'a, C, H> PathGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    pub fn new(context: &'a mut C, segment: ValidationPathSegment) -> Self {
        context.push_path(segment);
        Self {
            context,
            _phantom: PhantomData,
        }
    }
}

impl<'a, C, H> core::ops::Deref for PathGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<'a, C, H> core::ops::DerefMut for PathGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

impl<'a, C, H> Drop for PathGuard<'a, C, H>
where
    C: ValidationContext<H> + ?Sized,
    H: ArchiveHeaderTrait,
{
    fn drop(&mut self) {
        self.context.pop_path();
    }
}
