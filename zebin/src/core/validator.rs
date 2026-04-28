use core::{marker::PhantomData, num::NonZeroUsize};

use alloc::string::ToString;

use crate::{
    ZebinError,
    access::ResolvedLayout,
    core::schema::{LayoutDirectory, SchemaRevision, StableSchemaKey},
    traits::{ValidationContext, ValidationPathSegment},
};

/// Validator for byte streams to ensure safety before access.
pub struct Validator<'a> {
    data: &'a [u8],
    layouts: Option<LayoutDirectory<'a>>,
    cached_layout: Option<(StableSchemaKey, SchemaRevision, ResolvedLayout<'a>)>,
    depth: usize,
    max_depth: usize,
    path: alloc::vec::Vec<ValidationPathSegment>,
}

pub struct DepthGuard<'v> {
    context: *mut Validator<'v>,
    _phantom: PhantomData<&'v mut Validator<'v>>,
}

impl<'v> Drop for DepthGuard<'v> {
    fn drop(&mut self) {
        unsafe {
            (*self.context).pop_depth();
        }
    }
}

impl<'a> Validator<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            layouts: None,
            cached_layout: None,
            depth: 0,
            max_depth: 256,
            path: alloc::vec::Vec::new(),
        }
    }

    pub fn with_layouts(data: &'a [u8], layouts: LayoutDirectory<'a>) -> Self {
        Self {
            data,
            layouts: Some(layouts),
            cached_layout: None,
            depth: 0,
            max_depth: 256,
            path: alloc::vec::Vec::new(),
        }
    }

    /// Enter a nested validation scope and automatically unwind on drop.
    pub fn enter(&mut self) -> Result<DepthGuard<'a>, ZebinError> {
        self.push_depth()?;
        Ok(DepthGuard {
            context: self as *mut _,
            _phantom: PhantomData,
        })
    }

    /// Check if a pointer and its associated size are within the buffer bounds.
    pub fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError> {
        if size == 0 {
            return Ok(());
        }

        let start = ptr as usize;
        let buffer_start = self.data.as_ptr() as usize;
        let buffer_end = buffer_start
            .checked_add(self.data.len())
            .ok_or_else(|| self.validation_error("Buffer length overflow".to_string(), 0))?;

        let end = start.checked_add(size).ok_or_else(|| {
            self.validation_error(
                "Pointer range overflow".to_string(),
                start.saturating_sub(buffer_start),
            )
        })?;

        if start < buffer_start || end > buffer_end {
            return Err(self.validation_error(
                "Pointer out of bounds".to_string(),
                start.saturating_sub(buffer_start),
            ));
        }
        Ok(())
    }

    /// Check if a pointer is properly aligned.
    pub fn check_alignment(
        &self,
        ptr: *const u8,
        alignment: NonZeroUsize,
    ) -> Result<(), ZebinError> {
        let alignment_value = alignment.get();
        let addr = ptr as usize;
        if !addr.is_multiple_of(alignment_value) {
            return Err(ZebinError::AlignmentError {
                expected: alignment,
                actual: unsafe { NonZeroUsize::new_unchecked(addr % alignment_value) },
                pos: addr.saturating_sub(self.data.as_ptr() as usize),
            });
        }
        Ok(())
    }

    /// Push to recursion depth.
    pub fn push_depth(&mut self) -> Result<(), ZebinError> {
        self.depth += 1;
        if self.depth > self.max_depth {
            self.depth = self.depth.saturating_sub(1);
            return Err(ZebinError::RecursionLimitExceeded);
        }
        Ok(())
    }

    /// Pop from recursion depth.
    pub fn pop_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Get the underlying data slice.
    pub fn data(&self) -> &[u8] {
        self.data
    }

    pub fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'a>, ZebinError> {
        if let Some((cached_key, cached_revision, resolved)) = self.cached_layout
            && cached_key == stable_schema_key
            && cached_revision == schema_revision
        {
            return Ok(resolved);
        }

        let layouts = self.layouts.ok_or_else(|| ZebinError::ValidationError {
            message: "Missing layout directory".to_string(),
            pos: 0,
        })?;
        let header = crate::format::ArchiveHeader::parse(self.data)?;
        let layout = layouts.lookup(stable_schema_key, schema_revision)?;
        let resolved = ResolvedLayout::from_parts(self.data, header, layout);
        self.cached_layout = Some((stable_schema_key, schema_revision, resolved));
        Ok(resolved)
    }
}

impl<'a> ValidationContext for Validator<'a> {
    fn push_depth(&mut self) -> Result<(), ZebinError> {
        Validator::push_depth(self)
    }

    fn pop_depth(&mut self) {
        Validator::pop_depth(self)
    }

    fn check_range(&self, ptr: *const u8, size: usize) -> Result<(), ZebinError> {
        Validator::check_range(self, ptr, size)
    }

    fn check_alignment(&self, ptr: *const u8, alignment: NonZeroUsize) -> Result<(), ZebinError> {
        Validator::check_alignment(self, ptr, alignment)
    }

    fn push_path_segment_raw(&mut self, segment: ValidationPathSegment) {
        self.path.push(segment);
    }

    fn pop_path_segment(&mut self) {
        self.path.pop();
    }

    fn path(&self) -> &[ValidationPathSegment] {
        &self.path
    }

    fn resolved_layout(
        &mut self,
        stable_schema_key: StableSchemaKey,
        schema_revision: SchemaRevision,
    ) -> Result<ResolvedLayout<'a>, ZebinError> {
        Validator::resolved_layout(self, stable_schema_key, schema_revision)
    }
}
