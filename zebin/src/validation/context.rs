use crate::{
    ZebinError,
    core::schema::{SchemaRevision, StableSchemaKey},
    read::view::ResolvedLayout,
};
use alloc::string::String;
use core::num::NonZeroUsize;

/// A single segment in a validation path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValidationPathSegment {
    Field(&'static str),
    Index(usize),
    Variant(&'static str),
}

/// RAII guard that restores validation path state when dropped.
pub struct ValidationPathGuard<'a, C: ValidationContext + ?Sized> {
    context: &'a mut C,
}

impl<'a, C: ValidationContext + ?Sized> Drop for ValidationPathGuard<'a, C> {
    fn drop(&mut self) {
        self.context.pop_path_segment();
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::Deref for ValidationPathGuard<'a, C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::DerefMut for ValidationPathGuard<'a, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

/// Validation context used by archived representations.
pub trait ValidationContext {
    fn push_depth(&mut self) -> Result<(), ZebinError>;

    fn pop_depth(&mut self);

    fn guard(&mut self) -> Result<ArchivedDepthGuard<'_, Self>, ZebinError> {
        ArchivedDepthGuard::new(self)
    }

    fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError>;

    fn check_alignment(&self, ptr: *const u8, alignment: NonZeroUsize) -> Result<(), ZebinError>;

    fn push_path_segment(
        &mut self,
        segment: ValidationPathSegment,
    ) -> Result<ValidationPathGuard<'_, Self>, ZebinError> {
        self.push_path_segment_raw(segment);
        Ok(ValidationPathGuard { context: self })
    }

    fn push_path_segment_raw(&mut self, segment: ValidationPathSegment);

    fn pop_path_segment(&mut self);

    fn path(&self) -> &[ValidationPathSegment];

    fn validation_error(&self, message: impl Into<String>, pos: usize) -> ZebinError {
        let path = self.path();
        if path.is_empty() {
            ZebinError::ValidationError {
                message: message.into(),
                pos,
            }
        } else {
            ZebinError::ValidationError {
                message: format_path_message(path, message.into()),
                pos,
            }
        }
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'_>, ZebinError>;
}

/// RAII guard that restores validation depth when dropped.
pub struct ArchivedDepthGuard<'a, C: ValidationContext + ?Sized> {
    context: &'a mut C,
}

impl<'a, C: ValidationContext + ?Sized> ArchivedDepthGuard<'a, C> {
    pub fn new(context: &'a mut C) -> Result<Self, ZebinError> {
        context.push_depth()?;
        Ok(Self { context })
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

impl<'a, C: ValidationContext + ?Sized> core::ops::Deref for ArchivedDepthGuard<'a, C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        self.context
    }
}

impl<'a, C: ValidationContext + ?Sized> core::ops::DerefMut for ArchivedDepthGuard<'a, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.context
    }
}

impl<'a, C: ValidationContext + ?Sized> Drop for ArchivedDepthGuard<'a, C> {
    fn drop(&mut self) {
        self.context.pop_depth();
    }
}

fn format_path_message(path: &[ValidationPathSegment], message: String) -> String {
    use alloc::string::ToString;

    let mut prefix = String::new();
    for segment in path {
        match segment {
            ValidationPathSegment::Field(name) => {
                if !prefix.is_empty() {
                    prefix.push('.');
                }
                prefix.push_str(name);
            }
            ValidationPathSegment::Index(index) => {
                prefix.push('[');
                prefix.push_str(&index.to_string());
                prefix.push(']');
            }
            ValidationPathSegment::Variant(name) => {
                if !prefix.is_empty() {
                    prefix.push_str("::");
                }
                prefix.push_str(name);
            }
        }
    }

    if prefix.is_empty() {
        message
    } else {
        alloc::format!("{prefix}: {message}")
    }
}
