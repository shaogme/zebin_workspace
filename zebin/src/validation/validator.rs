use core::{marker::PhantomData, mem::MaybeUninit, num::NonZeroUsize};

#[cfg(feature = "alloc")]
use crate::alloc::vec::Vec;
use crate::{
    core::schema::{LayoutDirectory, SchemaRevision, StableSchemaKey},
    error::ValidateError,
    format::ArchiveHeader,
    read::ResolvedLayout,
    traits::ArchiveHeader as ArchiveHeaderTrait,
    validation::context::ValidationContext,
};

/// Validator for byte streams to ensure safety before access.
pub struct Validator<'a, H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    data: &'a [u8],
    header: H,
    layouts: Option<LayoutDirectory<'a>>,
    cached_layout: Option<(StableSchemaKey, SchemaRevision, ResolvedLayout<'a, H>)>,
    depth: usize,
    max_depth: usize,
    path: ValidationPathStack,
    last_error_path: Option<ValidationPathStack>,
}

pub struct DepthGuard<'v, H = ArchiveHeader>
where
    H: ArchiveHeaderTrait,
{
    context: *mut Validator<'v, H>,
    _phantom: PhantomData<&'v mut Validator<'v, H>>,
}

impl<'v, H: ArchiveHeaderTrait> Drop for DepthGuard<'v, H> {
    fn drop(&mut self) {
        unsafe {
            (*self.context).pop_depth();
        }
    }
}

impl<'a, H: ArchiveHeaderTrait> Validator<'a, H> {
    pub fn new(data: &'a [u8], header: H) -> Self {
        Self {
            data,
            header,
            layouts: None,
            cached_layout: None,
            depth: 0,
            max_depth: 128,
            path: ValidationPathStack::new(),
            last_error_path: None,
        }
    }

    pub fn with_layouts(data: &'a [u8], header: H, layouts: LayoutDirectory<'a>) -> Self {
        Self {
            data,
            header,
            layouts: Some(layouts),
            cached_layout: None,
            depth: 0,
            max_depth: 128,
            path: ValidationPathStack::new(),
            last_error_path: None,
        }
    }

    /// Enter a nested validation scope and automatically unwind on drop.
    pub fn enter(&mut self) -> Result<DepthGuard<'a, H>, ValidateError> {
        self.push_depth()?;
        Ok(DepthGuard {
            context: self as *mut _,
            _phantom: PhantomData,
        })
    }

    pub fn check_range(&mut self, ptr: *const u8, size: usize) -> Result<(), ValidateError> {
        if size == 0 {
            return Ok(());
        }

        let start = ptr as usize;
        let buffer_start = self.data.as_ptr() as usize;
        let buffer_end = buffer_start
            .checked_add(self.data.len())
            .ok_or_else(|| self.validation_error("Buffer length overflow", 0))?;

        let end = start.checked_add(size).ok_or_else(|| {
            self.validation_error("Pointer range overflow", start.saturating_sub(buffer_start))
        })?;

        if start < buffer_start || end > buffer_end {
            return Err(
                self.validation_error("Pointer out of bounds", start.saturating_sub(buffer_start))
            );
        }
        Ok(())
    }

    /// Check if a pointer is properly aligned.
    pub fn check_alignment(
        &mut self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ValidateError> {
        let alignment_value = alignment.get();
        let addr = ptr as usize;
        if !addr.is_multiple_of(alignment_value) {
            return Err(ValidateError::AlignmentError {
                expected: alignment,
                actual: unsafe { NonZeroUsize::new_unchecked(addr % alignment_value) },
                pos: addr.saturating_sub(self.data.as_ptr() as usize),
            });
        }
        Ok(())
    }

    /// Push to recursion depth.
    pub fn push_depth(&mut self) -> Result<(), ValidateError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth = self.depth.saturating_sub(1);
            return Err(ValidateError::RecursionLimitExceeded);
        }
        Ok(())
    }

    /// Pop from recursion depth.
    pub fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Push to validation path.
    pub fn push_path(&mut self, segment: ValidationPathSegment) {
        self.path.push(segment);
    }

    /// Pop from validation path.
    pub fn pop_path(&mut self) {
        self.path.pop();
    }

    pub fn record_error_path(&mut self) {
        if self.last_error_path.is_none() {
            self.last_error_path = Some(self.path.clone());
        }
    }

    pub fn last_error_path(&self) -> Option<&ValidationPathStack> {
        self.last_error_path.as_ref()
    }

    /// Get the underlying data slice.
    pub fn data(&self) -> &[u8] {
        self.data
    }

    pub fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'a, H>, ValidateError> {
        if let Some((cached_key, cached_revision, resolved)) = &self.cached_layout
            && *cached_key == stable_schema_key
            && *cached_revision == schema_revision
        {
            return Ok(*resolved);
        }

        let layouts = self.layouts.ok_or(ValidateError::ValidationError {
            message: "Missing layout directory",
            pos: 0,
        })?;
        let layout = layouts.lookup(stable_schema_key, schema_revision)?;
        let resolved = ResolvedLayout::from_parts(self.data, self.header, Some(layout));
        self.cached_layout = Some((stable_schema_key, schema_revision, resolved));
        Ok(resolved)
    }

    fn validation_error(&mut self, message: &'static str, pos: usize) -> ValidateError {
        self.record_error_path();
        ValidateError::ValidationError { message, pos }
    }
}

impl<'a, H: ArchiveHeaderTrait> ValidationContext<H> for Validator<'a, H> {
    fn push_depth(&mut self) -> Result<(), ValidateError> {
        Validator::push_depth(self)
    }

    fn pop_depth(&mut self) {
        Validator::pop_depth(self)
    }

    fn push_path(&mut self, segment: ValidationPathSegment) {
        Validator::push_path(self, segment)
    }

    fn pop_path(&mut self) {
        Validator::pop_path(self)
    }

    fn record_error_path(&mut self) {
        Validator::record_error_path(self)
    }

    fn check_range(&mut self, ptr: *const u8, size: usize) -> Result<(), ValidateError> {
        Validator::check_range(self, ptr, size)
    }

    fn check_alignment(
        &mut self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ValidateError> {
        Validator::check_alignment(self, ptr, alignment)
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'a, H>, ValidateError> {
        Validator::resolved_layout(self, stable_schema_key, schema_revision)
    }
}

/// A single segment in a validation path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationPathSegment {
    Field(&'static str),
    Index(usize),
    Variant(&'static str),
}

/// A fixed-capacity stack for validation path segments.
#[derive(Clone)]
pub struct ValidationPathStack {
    segments: [MaybeUninit<ValidationPathSegment>; 32],
    len: usize,
    #[cfg(feature = "alloc")]
    extra: Option<Vec<ValidationPathSegment>>,
}

impl Default for ValidationPathStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationPathStack {
    pub const fn new() -> Self {
        Self {
            segments: [MaybeUninit::uninit(); 32],
            len: 0,
            #[cfg(feature = "alloc")]
            extra: None,
        }
    }

    pub fn push(&mut self, segment: ValidationPathSegment) {
        if self.len < 32 {
            self.segments[self.len].write(segment);
            self.len += 1;
        } else {
            #[cfg(feature = "alloc")]
            {
                if let Some(extra) = &mut self.extra {
                    extra.push(segment);
                } else {
                    self.extra = Some(crate::alloc::vec![segment]);
                }
            }
        }
    }

    pub fn pop(&mut self) {
        #[cfg(feature = "alloc")]
        if let Some(extra) = &mut self.extra
            && extra.pop().is_some()
        {
            if extra.is_empty() {
                self.extra = None;
            }
            return;
        }

        if self.len > 0 {
            self.len -= 1;
        }
    }

    pub fn format(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut first = true;

        #[cfg(feature = "alloc")]
        if let Some(extra) = &self.extra {
            for segment in extra.iter().rev() {
                Self::format_segment(f, segment, &mut first)?;
            }
        }

        for i in (0..self.len).rev() {
            let segment = unsafe { self.segments[i].assume_init_ref() };
            Self::format_segment(f, segment, &mut first)?;
        }
        Ok(())
    }

    fn format_segment(
        f: &mut core::fmt::Formatter<'_>,
        segment: &ValidationPathSegment,
        first: &mut bool,
    ) -> core::fmt::Result {
        match segment {
            ValidationPathSegment::Field(name) => {
                if !*first {
                    write!(f, ".")?;
                }
                write!(f, "{}", name)?;
            }
            ValidationPathSegment::Index(index) => write!(f, "[{}]", index)?,
            ValidationPathSegment::Variant(name) => write!(f, "({})", name)?,
        }
        *first = false;
        Ok(())
    }
}
