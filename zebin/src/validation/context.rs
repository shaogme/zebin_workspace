use crate::{error::DecodeError, validation::path::ValidationPathSegment};
use core::num::NonZeroUsize;

/// Validation context used while sequentially decoding archive data.
pub trait ValidationContext {
    fn push_depth(&mut self) -> Result<(), DecodeError>;

    fn pop_depth(&mut self);

    fn push_path(&mut self, segment: ValidationPathSegment);

    fn pop_path(&mut self);

    fn record_error_path(&mut self);

    fn guard(&mut self) -> Result<ArchivedDepthGuard<'_, Self>, DecodeError> {
        ArchivedDepthGuard::new(self)
    }

    fn push_field(&mut self, name: &'static str) -> PathGuard<'_, Self> {
        PathGuard::new(self, ValidationPathSegment::Field(name))
    }

    fn push_index(&mut self, index: usize) -> PathGuard<'_, Self> {
        PathGuard::new(self, ValidationPathSegment::Index(index))
    }

    fn push_variant(&mut self, name: &'static str) -> PathGuard<'_, Self> {
        PathGuard::new(self, ValidationPathSegment::Variant(name))
    }

    fn check_range(&mut self, pos: usize, size: usize) -> Result<(), DecodeError>;

    fn check_alignment(&mut self, pos: usize, alignment: NonZeroUsize) -> Result<(), DecodeError>;

    fn error(&mut self, error: DecodeError) -> DecodeError {
        self.record_error_path();
        error
    }

    fn validation_error(&mut self, message: &'static str, pos: usize) -> DecodeError {
        self.error(DecodeError::ValidationError { message, pos })
    }
}

/// RAII guard that restores validation depth when dropped.
pub struct ArchivedDepthGuard<'a, C: ValidationContext + ?Sized> {
    context: &'a mut C,
}

impl<'a, C> ArchivedDepthGuard<'a, C>
where
    C: ValidationContext + ?Sized,
{
    pub fn new(context: &'a mut C) -> Result<Self, DecodeError> {
        context.push_depth()?;
        Ok(Self { context })
    }

    pub fn check_range(&mut self, pos: usize, size: usize) -> Result<(), DecodeError> {
        self.context.check_range(pos, size)
    }

    pub fn check_alignment(
        &mut self,
        pos: usize,
        alignment: NonZeroUsize,
    ) -> Result<(), DecodeError> {
        self.context.check_alignment(pos, alignment)
    }
}

impl<C> core::ops::Deref for ArchivedDepthGuard<'_, C>
where
    C: ValidationContext + ?Sized,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<C> core::ops::DerefMut for ArchivedDepthGuard<'_, C>
where
    C: ValidationContext + ?Sized,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

impl<C> Drop for ArchivedDepthGuard<'_, C>
where
    C: ValidationContext + ?Sized,
{
    fn drop(&mut self) {
        self.context.pop_depth();
    }
}

/// RAII guard that restores validation path when dropped.
pub struct PathGuard<'a, C: ValidationContext + ?Sized> {
    context: &'a mut C,
}

impl<'a, C> PathGuard<'a, C>
where
    C: ValidationContext + ?Sized,
{
    pub fn new(context: &'a mut C, segment: ValidationPathSegment) -> Self {
        context.push_path(segment);
        Self { context }
    }
}

impl<C> core::ops::Deref for PathGuard<'_, C>
where
    C: ValidationContext + ?Sized,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<C> core::ops::DerefMut for PathGuard<'_, C>
where
    C: ValidationContext + ?Sized,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

impl<C> Drop for PathGuard<'_, C>
where
    C: ValidationContext + ?Sized,
{
    fn drop(&mut self) {
        self.context.pop_path();
    }
}
