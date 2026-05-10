use crate::{
    ZebinError,
    core::schema::{SchemaRevision, StableSchemaKey},
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
    fn push_depth(&mut self) -> Result<(), ZebinError>;

    fn pop_depth(&mut self);

    fn guard(&mut self) -> Result<ArchivedDepthGuard<'_, Self, H>, ZebinError> {
        ArchivedDepthGuard::new(self)
    }

    fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError>;

    fn check_alignment(&self, ptr: *const u8, alignment: NonZeroUsize) -> Result<(), ZebinError>;

    fn validation_error(&self, message: &'static str, pos: usize) -> ZebinError {
        ZebinError::ValidationError {
            message,
            pos,
            path: crate::error::ValidationPathStack::new(),
        }
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'_, H>, ZebinError>;
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
    pub fn new(context: &'a mut C) -> Result<Self, ZebinError> {
        context.push_depth()?;
        Ok(Self {
            context,
            _phantom: PhantomData,
        })
    }

    pub fn check_range(&mut self, ptr: *const u8, size: usize) -> Result<(), ZebinError> {
        self.context.check_range(ptr, size)
    }

    pub fn check_alignment(
        &mut self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ZebinError> {
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
